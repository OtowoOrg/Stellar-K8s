import { SorobanRpc, xdr, Contract, Address } from '@stellar/stellar-sdk';

/**
 * Parsed WASM exported function
 */
export interface WasmExport {
  name: string;
  kind: string;
  params: any[];
  returns: any[];
}

/**
 * Fetch a deployed contract's WASM bytecode using its Contract ID.
 * @param rpcUrl Soroban RPC URL
 * @param contractId Contract ID
 * @returns ArrayBuffer of the WASM bytecode
 */
export async function fetchContractWasm(rpcUrl: string, contractId: string): Promise<ArrayBuffer> {
  const server = new SorobanRpc.Server(rpcUrl, { allowHttp: true });

  // Get contract entry
  const contract = new Contract(contractId);
  const ledgerKey = xdr.LedgerKey.contractData(
    new xdr.LedgerKeyContractData({
      contract: contract.address().toScAddress(),
      key: xdr.ScVal.scvLedgerKeyContractInstance(),
      durability: xdr.ContractDataDurability.persistent(),
    })
  );

  const entry = await server.getLedgerEntries(ledgerKey);
  if (!entry.entries || entry.entries.length === 0) {
    throw new Error('Contract not found on ledger');
  }

  const ledgerEntry = entry.entries[0].val;
  const contractData = ledgerEntry.contractData();
  const contractInstance = contractData.val().instance();

  const executable = contractInstance.executable();
  if (executable.switch() !== xdr.ContractExecutableType.contractExecutableWasm()) {
    throw new Error('Contract is not a WASM executable');
  }

  const wasmHash = executable.wasmHash();
  
  // Get WASM bytecode entry
  const wasmKey = xdr.LedgerKey.contractCode(
    new xdr.LedgerKeyContractCode({
      hash: wasmHash,
    })
  );

  const wasmEntryResponse = await server.getLedgerEntries(wasmKey);
  if (!wasmEntryResponse.entries || wasmEntryResponse.entries.length === 0) {
    throw new Error('WASM bytecode not found');
  }

  const wasmEntry = wasmEntryResponse.entries[0].val;
  const wasmCode = wasmEntry.contractCode().code();

  return wasmCode.buffer;
}

/**
 * Parse WASM bytecode to extract exported functions.
 * Relies on the standard WebAssembly API for parsing.
 * @param wasmCode ArrayBuffer of the WASM bytecode
 * @returns Array of WasmExport
 */
export async function parseWasmExports(wasmCode: ArrayBuffer): Promise<WasmExport[]> {
  const module = await WebAssembly.compile(wasmCode);
  const exports = WebAssembly.Module.exports(module);

  // We map the native WASM exports to our interface
  // Parameter and return types would require parsing the Custom Sections or
  // using @stellar/stellar-sdk ContractSpec if it exists, but for basic 
  // exported function names, WebAssembly.Module.exports is sufficient and runs natively.
  
  const parsedExports: WasmExport[] = exports
    .filter(e => e.kind === 'function' && e.name !== 'update_current_memory_base')
    .map(e => ({
      name: e.name,
      kind: e.kind,
      params: [], // Types would require deeper custom section parsing
      returns: [], // Types would require deeper custom section parsing
    }));

  return parsedExports;
}

/**
 * Fetch TTL metadata for a contract
 * @param rpcUrl 
 * @param contractId 
 */
export async function getContractTtl(rpcUrl: string, contractId: string) {
  const server = new SorobanRpc.Server(rpcUrl, { allowHttp: true });
  const contract = new Contract(contractId);
  const ledgerKey = xdr.LedgerKey.contractData(
    new xdr.LedgerKeyContractData({
      contract: contract.address().toScAddress(),
      key: xdr.ScVal.scvLedgerKeyContractInstance(),
      durability: xdr.ContractDataDurability.persistent(),
    })
  );

  const entry = await server.getLedgerEntries(ledgerKey);
  if (!entry.entries || entry.entries.length === 0) {
    return null;
  }
  
  return entry.entries[0].liveUntilLedgerSeq;
}
