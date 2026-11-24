// Copyright 2026 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Constrained CEL subset compiled once and evaluated in-process.
//!
//! Kubernetes admission is moving to CEL (`ValidatingAdmissionPolicy`).
//! This evaluator implements the boolean/comparison subset used by Stellar
//! admission policies so evaluation stays well under the 5ms p99 budget.

use crate::error::{Error, Result};

/// Compiled CEL expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// Literal value.
    Lit(Literal),
    /// Field path (`object.spec.network`).
    Path(Path),
    /// `has(path)` presence check.
    Has(Path),
    /// Logical not.
    Not(Box<Expr>),
    /// Logical and.
    And(Box<Expr>, Box<Expr>),
    /// Logical or.
    Or(Box<Expr>, Box<Expr>),
    /// Comparison.
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
}

/// Comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// CEL literal.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    String(String),
}

/// Dotted / indexed object path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path(pub Vec<PathSeg>);

/// One path segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSeg {
    Ident(String),
    Index(String),
}

/// Parse a CEL expression into an AST. Invalid syntax fails closed.
pub fn parse(input: &str) -> Result<Expr> {
    let mut p = Parser {
        tokens: tokenize(input)?,
        pos: 0,
    };
    let expr = p.parse_or()?;
    if p.peek().is_some() {
        return Err(Error::ValidationError(format!(
            "malformed CEL: trailing tokens in `{input}`"
        )));
    }
    Ok(expr)
}

/// Evaluate a compiled expression against a JSON admission object.
pub fn eval(expr: &Expr, root: &serde_json::Value) -> Result<bool> {
    match coerce_bool(&eval_value(expr, root)?) {
        Some(b) => Ok(b),
        None => Err(Error::ValidationError(
            "CEL expression did not produce a boolean".to_string(),
        )),
    }
}

fn eval_value(expr: &Expr, root: &serde_json::Value) -> Result<serde_json::Value> {
    match expr {
        Expr::Lit(Literal::Null) => Ok(serde_json::Value::Null),
        Expr::Lit(Literal::Bool(b)) => Ok(serde_json::Value::Bool(*b)),
        Expr::Lit(Literal::Int(i)) => Ok(serde_json::json!(*i)),
        Expr::Lit(Literal::String(s)) => Ok(serde_json::Value::String(s.clone())),
        Expr::Path(path) => Ok(lookup(root, path)
            .cloned()
            .unwrap_or(serde_json::Value::Null)),
        Expr::Has(path) => Ok(serde_json::Value::Bool(lookup(root, path).is_some())),
        Expr::Not(inner) => Ok(serde_json::Value::Bool(!eval(inner, root)?)),
        Expr::And(l, r) => Ok(serde_json::Value::Bool(eval(l, root)? && eval(r, root)?)),
        Expr::Or(l, r) => Ok(serde_json::Value::Bool(eval(l, root)? || eval(r, root)?)),
        Expr::Cmp(l, op, r) => {
            let lv = eval_value(l, root)?;
            let rv = eval_value(r, root)?;
            compare(&lv, *op, &rv)
                .map(serde_json::Value::Bool)
                .ok_or_else(|| Error::ValidationError("incomparable CEL values".to_string()))
        }
    }
}

fn lookup<'a>(root: &'a serde_json::Value, path: &Path) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for seg in &path.0 {
        match seg {
            PathSeg::Ident(k) | PathSeg::Index(k) => {
                cur = cur.get(k)?;
            }
        }
    }
    Some(cur)
}

fn coerce_bool(v: &serde_json::Value) -> Option<bool> {
    match v {
        serde_json::Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn compare(l: &serde_json::Value, op: CmpOp, r: &serde_json::Value) -> Option<bool> {
    if matches!(op, CmpOp::Eq | CmpOp::Ne) {
        let eq = json_eq(l, r);
        return Some(if matches!(op, CmpOp::Eq) { eq } else { !eq });
    }
    let (li, ri) = (as_i64(l)?, as_i64(r)?);
    Some(match op {
        CmpOp::Lt => li < ri,
        CmpOp::Le => li <= ri,
        CmpOp::Gt => li > ri,
        CmpOp::Ge => li >= ri,
        CmpOp::Eq | CmpOp::Ne => unreachable!(),
    })
}

fn json_eq(l: &serde_json::Value, r: &serde_json::Value) -> bool {
    if let (Some(a), Some(b)) = (as_i64(l), as_i64(r)) {
        return a == b;
    }
    l == r
}

fn as_i64(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Ident(String),
    String(String),
    Int(i64),
    True,
    False,
    Null,
    And,
    Or,
    Not,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Comma,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Result<Token> {
        let t =
            self.tokens.get(self.pos).cloned().ok_or_else(|| {
                Error::ValidationError("malformed CEL: unexpected end".to_string())
            })?;
        self.pos += 1;
        Ok(t)
    }

    fn eat(&mut self, expected: &Token) -> Result<()> {
        let got = self.bump()?;
        if &got == expected {
            Ok(())
        } else {
            Err(Error::ValidationError(format!(
                "malformed CEL: expected {expected:?}, got {got:?}"
            )))
        }
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.pos += 1;
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.parse_not()?)));
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr> {
        let left = self.parse_primary()?;
        let op = match self.peek() {
            Some(Token::Eq) => CmpOp::Eq,
            Some(Token::Ne) => CmpOp::Ne,
            Some(Token::Lt) => CmpOp::Lt,
            Some(Token::Le) => CmpOp::Le,
            Some(Token::Gt) => CmpOp::Gt,
            Some(Token::Ge) => CmpOp::Ge,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.parse_primary()?;
        Ok(Expr::Cmp(Box::new(left), op, Box::new(right)))
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.bump()? {
            Token::True => Ok(Expr::Lit(Literal::Bool(true))),
            Token::False => Ok(Expr::Lit(Literal::Bool(false))),
            Token::Null => Ok(Expr::Lit(Literal::Null)),
            Token::Int(i) => Ok(Expr::Lit(Literal::Int(i))),
            Token::String(s) => Ok(Expr::Lit(Literal::String(s))),
            Token::LParen => {
                let inner = self.parse_or()?;
                self.eat(&Token::RParen)?;
                Ok(inner)
            }
            Token::Ident(name) if name == "has" => {
                self.eat(&Token::LParen)?;
                let ident = self.expect_ident()?;
                let path = self.parse_path_from_ident(ident)?;
                self.eat(&Token::RParen)?;
                Ok(Expr::Has(path))
            }
            Token::Ident(name) => Ok(Expr::Path(self.parse_path_from_ident(name)?)),
            other => Err(Error::ValidationError(format!(
                "malformed CEL: unexpected token {other:?}"
            ))),
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        match self.bump()? {
            Token::Ident(s) => Ok(s),
            other => Err(Error::ValidationError(format!(
                "malformed CEL: expected identifier, got {other:?}"
            ))),
        }
    }

    fn parse_path_from_ident(&mut self, first: String) -> Result<Path> {
        let mut segs = vec![PathSeg::Ident(first)];
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.pos += 1;
                    segs.push(PathSeg::Ident(self.expect_ident()?));
                }
                Some(Token::LBracket) => {
                    self.pos += 1;
                    match self.bump()? {
                        Token::String(s) => segs.push(PathSeg::Index(s)),
                        other => {
                            return Err(Error::ValidationError(format!(
                                "malformed CEL: expected string index, got {other:?}"
                            )))
                        }
                    }
                    self.eat(&Token::RBracket)?;
                }
                _ => break,
            }
        }
        Ok(Path(segs))
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i + 1]);
                    i += 2;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            if i >= chars.len() {
                return Err(Error::ValidationError(
                    "malformed CEL: unterminated string".to_string(),
                ));
            }
            i += 1;
            tokens.push(Token::String(s));
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            if c == '-' {
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let n: i64 = input[start..i].parse().map_err(|_| {
                Error::ValidationError("malformed CEL: invalid integer".to_string())
            })?;
            tokens.push(Token::Int(n));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident = &input[start..i];
            tokens.push(match ident {
                "true" => Token::True,
                "false" => Token::False,
                "null" => Token::Null,
                _ => Token::Ident(ident.to_string()),
            });
            continue;
        }
        let two = if i + 1 < chars.len() {
            Some((c, chars[i + 1]))
        } else {
            None
        };
        match (c, two) {
            ('&', Some(('&', '&'))) => {
                tokens.push(Token::And);
                i += 2;
            }
            ('|', Some(('|', '|'))) => {
                tokens.push(Token::Or);
                i += 2;
            }
            ('=', Some(('=', '='))) => {
                tokens.push(Token::Eq);
                i += 2;
            }
            ('!', Some(('!', '='))) => {
                tokens.push(Token::Ne);
                i += 2;
            }
            ('<', Some(('<', '='))) => {
                tokens.push(Token::Le);
                i += 2;
            }
            ('>', Some(('>', '='))) => {
                tokens.push(Token::Ge);
                i += 2;
            }
            ('!', _) => {
                tokens.push(Token::Not);
                i += 1;
            }
            ('<', _) => {
                tokens.push(Token::Lt);
                i += 1;
            }
            ('>', _) => {
                tokens.push(Token::Gt);
                i += 1;
            }
            ('(', _) => {
                tokens.push(Token::LParen);
                i += 1;
            }
            (')', _) => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ('[', _) => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            (']', _) => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            ('.', _) => {
                tokens.push(Token::Dot);
                i += 1;
            }
            (',', _) => {
                tokens.push(Token::Comma);
                i += 1;
            }
            _ => {
                return Err(Error::ValidationError(format!(
                    "malformed CEL: unexpected character `{c}`"
                )))
            }
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_field_equality() {
        let expr = parse(r#"object.spec.network == "Testnet""#).unwrap();
        let root = serde_json::json!({"object":{"spec":{"network":"Testnet"}}});
        assert!(eval(&expr, &root).unwrap());
    }

    #[test]
    fn has_and_logic() {
        let expr =
            parse(r#"has(object.spec.validatorConfig) && object.spec.replicas <= 1"#).unwrap();
        let root = serde_json::json!({"object":{"spec":{"validatorConfig":{},"replicas":1}}});
        assert!(eval(&expr, &root).unwrap());
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse("object.spec.network ==").is_err());
    }
}
