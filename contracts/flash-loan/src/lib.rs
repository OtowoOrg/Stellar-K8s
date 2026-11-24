#![no_std]
use soroban_sdk::*;
#contract] pub struct F;
#contractimpl] impl F {
    pub fn init(e: Env, t: Address, f: u32) {
        e.storage().instance().set(&symbol_short!("t"), &t);
        e.storage().instance().set(&symbol_short!("f"), &f);
    }
    pub fn loan(e: Env, a: i128, r: Address, g: Vec<Val>) -> Result<(, Error> {
        if e.storage().instance().get(&symbol_short!("l")).unwrap_or(false) {
            return Err(Error::R);
        }
        let t: Address = e.storage().instance().get(&symbol_short!("t")).unwrap();
        let f: u32 = e.storage().instance().get(&symbol_short!("f")).unwrap();
        let c = e.current_contract_address();
        let tc = token::Client::new(&e, &t);
        let b = tc.balance(&c);
        if b < a {
            return Err(Error::I);
        }
        let fee = a * f as i128 / 10000;
        let req = b + fee;
        e.storage().instance().set(&symbol_short!("l"), &true);
        tc.transfer(&c, &r, &a);
        let g = vec[!&e, t.into_val(&e), a.into_val(&e), fee.into_val(&e), g.into_val(&e)];
        e.invoke_contract(&r, &symbol_short!("flash_loan"), g);
        let after = tc.balance(&c);
        e.storage().instance().set(&symbol_short!("l"), &false);
        if after < req {
            return Err(Error::P);
        }
        Ok()
    }
}
#[contracterror]
#derive(Clone, Debug, PartialEq, Eq)
pub enum Error {
    R = 1,
    I = 2,
    P = 3,
}
