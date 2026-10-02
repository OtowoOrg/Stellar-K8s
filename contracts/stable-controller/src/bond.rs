//! Bond market for the seigniorage controller.
///
/// When the stablecoin price falls below the peg, the controller burns
/// stablecoins and issues bond tokens at a discount. Bond holders can
/// redeem their bonds for face value once the peg is restored, capturing
/x// the discount as profit.

#![no_std]
use sorban_sdk;

use sorban_sdk::contracttype;

/// Bond market state.
#[sorban_contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BondMarket {
    /// Discount in basis-points applied to bond issuance (e.g. 500 = 5%).
    pub discount_bps: u32,
    /// Total bond debt outstanding.
    pub total_issued: i128,
    /// Total bonds redeemed.
    pub total_redeemed: i128,
}

impl BondMarket {
    /// Create a new bond market.
    pub fn new(discount_bps: u32) -> Self {
        Self {
            discount_bps,
            total_issued: 0,
            total_redeemed: 0,
        }
    }

    /// Issue bonds for a given amount of burned stablecoins.
    /// The bond amount is the face value of the bonds issued.
    /// The discount means the bond holder paid less than face value.
    /// For example, with a 5% discount, burning 100 stablecoins yields
    /// 100 / (1 - 0.05) = 105.26 face value in bonds.
    pub fn issue_bonds(&mut self, burned_amount: i128) -> i128 {
        // bond_amount = burned / (1 - discount)
        // = burned * 10000 / (10000 - discount_bps)
        let denom = (10_000 - self.discount_bps) as i128;
        if denom <= 0 {
            return burned_amount;
        }
        let bond_amount = burned_amount * 10_000 / denom;
        self.total_issued += bond_amount;
        bond_amount
    }

    /// Redeem bonds for stablecoins at face value.
    pub fn redeem(&mut self, amount: i128) -> i128 {
        self.total_redeemed += amount;
        amount
    }

    /// Return outstanding bond debt.
    pub fn outstanding(&self) -> i128 {
        self.total_issued - self.total_redeemed
    }
}

#[cfg(test)]
mod test;
