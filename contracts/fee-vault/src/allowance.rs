use soroban_sdk::Address;

#[derive(Clone)]
pub struct Allowance {
    pub operator: Address,
    pub limit: i128,
    pub spent: i128,
    pub window_start: u64,
    pub window_seconds: u64,
}

impl Allowance {
    pub fn new(
        operator: Address,
        limit: i128,
        window_start: u64,
        window_seconds: u64,
    ) -> Self {
        Self {
            operator,
            limit,
            spent: 0,
            window_start,
            window_seconds,
        }
    }

    pub fn available(&self, now: u64) -> i128 {
        if now >= self.window_start + self.window_seconds {
            self.limit
        } else {
            self.limit.saturating_sub(self.spent)
        }
    }

    pub fn consume(&mut self, amount: i128, now: u64) -> bool {
        if amount < 0 {
            return false;
        }

        if now >= self.window_start + self.window_seconds {
            self.window_start = now;
            self.spent = 0;
        }

        if self.spent.saturating_add(amount) > self.limit {
            return false;
        }

        self.spent += amount;
        true
    }
}