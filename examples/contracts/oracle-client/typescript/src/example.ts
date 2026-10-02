/**
 * Prints the oracle's current quote. Usage:
 *
 *   ORACLE_CONTRACT_ID=C... npm run example
 */
import { Networks } from "@stellar/stellar-sdk";

import { EMA_SCALE, GasOracleClient, OracleContractError, OracleError } from "./client.js";

const contractId = process.env.ORACLE_CONTRACT_ID;
if (!contractId) {
  console.error("Set ORACLE_CONTRACT_ID to the deployed oracle contract address.");
  process.exit(1);
}

const oracle = new GasOracleClient({
  rpcUrl: process.env.SOROBAN_RPC_URL ?? "https://soroban-testnet.stellar.org",
  networkPassphrase: process.env.NETWORK_PASSPHRASE ?? Networks.TESTNET,
  contractId,
});

const FALLBACK_FEE_PER_OP = 100n;

try {
  const [state, feePerOp, fiveOps] = await Promise.all([
    oracle.state(),
    oracle.feePerOp(),
    oracle.estimateFee(5),
  ]);
  const whole = state.ema / EMA_SCALE;
  const fraction = (state.ema % EMA_SCALE).toString().padStart(7, "0");
  console.log(`EMA:            ${whole}.${fraction} stroops/op (${state.samples} samples)`);
  console.log(`fee_per_op:     ${feePerOp} stroops`);
  console.log(`estimate_fee(5): ${fiveOps} stroops`);
} catch (err) {
  const stale =
    err instanceof OracleContractError &&
    (err.code === OracleError.Stale || err.code === OracleError.NoData);
  if (!stale) throw err;
  console.warn(`${err.message}; falling back to ${FALLBACK_FEE_PER_OP} stroops/op`);
}
