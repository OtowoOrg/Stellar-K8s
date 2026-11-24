use soroban_sdk::Env;

/// Calculates the required reserve token amount for minting `amount` of continuous tokens.
pub fn calculate_purchase_return(
    total_supply: i128,
    reserve_balance: i128,
    reserve_ratio: u32,
    amount: i128,
) -> i128 {
    if total_supply == 0 {
        return amount; // Base price of 1 when supply is 0. 
    }
    
    if reserve_ratio == 10000 {
        // Reserve Ratio 100%: Constant price. Cost = amount * (ReserveBalance / TotalSupply)
        return amount * reserve_balance / total_supply;
    }
    
    // Linear bonding curve approximation: ReserveRatio = 50% (5000)
    if reserve_ratio == 5000 {
        let ts = total_supply;
        let ts_plus_amount = total_supply + amount;
        
        let term1 = (ts_plus_amount * ts_plus_amount) / ts;
        let term2 = ts;
        
        return (reserve_balance * (term1 - term2)) / ts;
    }

    // Default fallback
    let ts = total_supply;
    let ts_plus_amount = total_supply + amount;
    (reserve_balance * (ts_plus_amount - ts)) / ts
}

/// Calculates the reserve token return for selling `amount` of continuous tokens.
pub fn calculate_sale_return(
    total_supply: i128,
    reserve_balance: i128,
    reserve_ratio: u32,
    amount: i128,
) -> i128 {
    if total_supply == 0 || amount == 0 {
        return 0;
    }
    
    if amount == total_supply {
        return reserve_balance;
    }

    if reserve_ratio == 10000 {
        return amount * reserve_balance / total_supply;
    }
    
    if reserve_ratio == 5000 {
        let ts = total_supply;
        let ts_minus_amount = total_supply - amount;
        
        let term1 = ts;
        let term2 = (ts_minus_amount * ts_minus_amount) / ts;
        
        return (reserve_balance * (term1 - term2)) / ts;
    }

    // Default fallback
    let ts = total_supply;
    let ts_minus_amount = total_supply - amount;
    (reserve_balance * (ts - ts_minus_amount)) / ts
}
