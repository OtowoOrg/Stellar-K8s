/**
 * Read-only TypeScript client for the Stellar-K8s gas fee oracle contract.
 *
 * Queries are served by Soroban RPC `simulateTransaction`, so they are free,
 * need no signature, and the source account does not have to exist on-chain.
 * The ABI mirrors docs/architecture/gas-oracle.md (section "Contract ABI").
 */
import {
  Account,
  BASE_FEE,
  Contract,
  Keypair,
  TransactionBuilder,
  nativeToScVal,
  rpc,
  scValToNative,
  xdr,
} from "@stellar/stellar-sdk";

/** Error codes surfaced by the contract as `Error(Contract, #n)`. */
export enum OracleError {
  AlreadyInitialized = 1,
  NotInitialized = 2,
  Unauthorized = 3,
  InvalidConfig = 4,
  ObservationOutOfRange = 5,
  AlreadyUpdated = 6,
  NoData = 7,
  Stale = 8,
  Overflow = 9,
}

export interface OracleConfig {
  alphaBps: number;
  minFee: bigint;
  maxFee: bigint;
  maxAgeLedgers: number;
}

export interface OracleState {
  /** EMA in fixed point, units of 10^-7 stroops. */
  ema: bigint;
  lastObservation: bigint;
  lastLedger: number;
  samples: bigint;
}

/** Scaling factor `S` of the fixed-point EMA. */
export const EMA_SCALE = 10_000_000n;

/** Thrown when the oracle rejects a call with one of its own error codes. */
export class OracleContractError extends Error {
  constructor(readonly code: OracleError) {
    super(`gas oracle error ${code} (${OracleError[code] ?? "Unknown"})`);
    this.name = "OracleContractError";
  }
}

const CONTRACT_ERROR = /Error\(Contract, #(\d+)\)/;

/** Extracts the oracle error code from a simulation error message, if present. */
export function parseOracleError(message: string): OracleError | undefined {
  const match = CONTRACT_ERROR.exec(message);
  return match ? (Number(match[1]) as OracleError) : undefined;
}

export interface GasOracleClientOptions {
  rpcUrl: string;
  contractId: string;
  networkPassphrase: string;
}

export class GasOracleClient {
  private readonly server: rpc.Server;
  private readonly contract: Contract;
  private readonly networkPassphrase: string;
  // Simulation only needs a syntactically valid source account.
  private readonly source = new Account(Keypair.random().publicKey(), "0");

  constructor({ rpcUrl, contractId, networkPassphrase }: GasOracleClientOptions) {
    this.server = new rpc.Server(rpcUrl, { allowHttp: rpcUrl.startsWith("http://") });
    this.contract = new Contract(contractId);
    this.networkPassphrase = networkPassphrase;
  }

  /** Recommended inclusion fee per operation, in stroops. */
  async feePerOp(): Promise<bigint> {
    return this.call<bigint>("fee_per_op");
  }

  /** Recommended total inclusion fee for `ops` operations, in stroops. */
  async estimateFee(ops: number): Promise<bigint> {
    return this.call<bigint>("estimate_fee", nativeToScVal(ops, { type: "u32" }));
  }

  async state(): Promise<OracleState> {
    const raw = await this.call<Record<string, bigint | number>>("state");
    return {
      ema: BigInt(raw.ema),
      lastObservation: BigInt(raw.last_observation),
      lastLedger: Number(raw.last_ledger),
      samples: BigInt(raw.samples),
    };
  }

  async config(): Promise<OracleConfig> {
    const raw = await this.call<Record<string, bigint | number>>("config");
    return {
      alphaBps: Number(raw.alpha_bps),
      minFee: BigInt(raw.min_fee),
      maxFee: BigInt(raw.max_fee),
      maxAgeLedgers: Number(raw.max_age_ledgers),
    };
  }

  private async call<T>(method: string, ...args: xdr.ScVal[]): Promise<T> {
    const tx = new TransactionBuilder(this.source, {
      fee: BASE_FEE,
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(this.contract.call(method, ...args))
      .setTimeout(30)
      .build();

    const sim = await this.server.simulateTransaction(tx);
    if (rpc.Api.isSimulationError(sim)) {
      const code = parseOracleError(sim.error);
      throw code === undefined ? new Error(sim.error) : new OracleContractError(code);
    }
    if (!sim.result) {
      throw new Error(`gas oracle ${method}: simulation returned no result`);
    }
    return scValToNative(sim.result.retval) as T;
  }
}
