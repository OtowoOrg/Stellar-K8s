#![no_main]
mod common;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: common::Input| common::run(input, common::Invariant::OwnerAuthorizesDebits));
