import { STELLAR_METRICS, OPERATORS } from '../lib/promqlGenerator.js';

const METRIC_NAMES = new Set(Object.keys(STELLAR_METRICS));
const KEYWORDS = new Set(['and', 'or', 'increase']);
const OPERATOR_SET = new Set(OPERATORS);

/**
 * Tokenize a PromQL expression line for highlighting.
 * Splits on whitespace and punctuation while keeping tokens meaningful.
 */
function tokenize(line) {
  return line.split(/(\s+|[()[\],])/).filter((t) => t.length > 0);
}

function classifyToken(token) {
  const trimmed = token.trim();
  if (!trimmed) return null;
  if (METRIC_NAMES.has(trimmed)) return 'token-metric';
  if (KEYWORDS.has(trimmed)) return 'token-keyword';
  if (OPERATOR_SET.has(trimmed)) return 'token-operator';
  if (/^-?\d+(\.\d+)?$/.test(trimmed)) return 'token-number';
  if (/^\d+[smhd]$/.test(trimmed)) return 'token-duration';
  return null;
}

export default function PromqlPreview({ expr, error }) {
  if (error) {
    return <div className="field-error">{error}</div>;
  }
  if (!expr) {
    return <pre className="promql-preview">// Add at least one condition</pre>;
  }

  return (
    <pre className="promql-preview">
      {expr.split('\n').map((line, lineIndex) => (
        <div key={lineIndex} className="promql-line">
          {tokenize(line).map((token, tokenIndex) => {
            const cls = classifyToken(token);
            return cls ? (
              <span key={tokenIndex} className={cls}>
                {token}
              </span>
            ) : (
              <span key={tokenIndex}>{token}</span>
            );
          })}
        </div>
      ))}
    </pre>
  );
}
