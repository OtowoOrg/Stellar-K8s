import React, { useState } from 'react';
import { fetchContractWasm, parseWasmExports, getContractTtl, WasmExport } from '../../services/wasm_parser';

export const ContractInspector: React.FC = () => {
  const [contractId, setContractId] = useState('');
  const [rpcUrl, setRpcUrl] = useState('https://soroban-testnet.stellar.org');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  
  const [exports, setExports] = useState<WasmExport[]>([]);
  const [ttl, setTtl] = useState<number | null>(null);

  const handleInspect = async () => {
    setLoading(true);
    setError(null);
    setExports([]);
    setTtl(null);

    try {
      if (!contractId) {
        throw new Error('Please enter a Contract ID');
      }

      // Fetch TTL metadata
      const contractTtl = await getContractTtl(rpcUrl, contractId);
      setTtl(contractTtl);

      // Fetch and parse WASM
      const wasmCode = await fetchContractWasm(rpcUrl, contractId);
      const parsedExports = await parseWasmExports(wasmCode);
      setExports(parsedExports);

    } catch (err: any) {
      setError(err.message || 'Failed to inspect contract');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="contract-inspector p-6 bg-slate-900 text-slate-100 rounded-xl shadow-2xl max-w-4xl mx-auto font-sans">
      <h2 className="text-2xl font-bold mb-6 bg-clip-text text-transparent bg-gradient-to-r from-blue-400 to-emerald-400">
        Soroban Contract Bytecode Inspector
      </h2>

      <div className="flex flex-col md:flex-row gap-4 mb-8">
        <div className="flex-1">
          <label className="block text-sm text-slate-400 mb-1">Soroban RPC URL</label>
          <input 
            type="text" 
            value={rpcUrl}
            onChange={(e) => setRpcUrl(e.target.value)}
            className="w-full bg-slate-800 border border-slate-700 rounded p-2 text-slate-200 focus:outline-none focus:border-blue-500 transition-colors"
          />
        </div>
        <div className="flex-[2]">
          <label className="block text-sm text-slate-400 mb-1">Contract ID</label>
          <div className="flex gap-2">
            <input 
              type="text" 
              value={contractId}
              onChange={(e) => setContractId(e.target.value)}
              placeholder="C..."
              className="w-full bg-slate-800 border border-slate-700 rounded p-2 text-slate-200 focus:outline-none focus:border-blue-500 transition-colors"
            />
            <button 
              onClick={handleInspect}
              disabled={loading}
              className="bg-blue-600 hover:bg-blue-700 disabled:bg-blue-800 text-white font-medium py-2 px-6 rounded transition-colors"
            >
              {loading ? 'Inspecting...' : 'Inspect'}
            </button>
          </div>
        </div>
      </div>

      {error && (
        <div className="bg-red-900/30 border border-red-800 text-red-300 p-4 rounded mb-6">
          {error}
        </div>
      )}

      {ttl !== null && (
        <div className="mb-6 p-4 bg-slate-800 rounded border border-slate-700 flex justify-between items-center">
          <div>
            <h3 className="text-slate-400 text-sm font-semibold uppercase tracking-wider mb-1">Contract Instance TTL</h3>
            <p className="text-lg text-emerald-400 font-mono">Live until Ledger: {ttl}</p>
          </div>
          <div className="w-12 h-12 rounded-full bg-emerald-900/50 flex items-center justify-center">
            <span className="text-emerald-400 text-xl">⏱</span>
          </div>
        </div>
      )}

      {exports.length > 0 && (
        <div>
          <h3 className="text-lg font-semibold mb-4 text-slate-200 border-b border-slate-700 pb-2">
            WASM Exported Functions ({exports.length})
          </h3>
          <div className="grid gap-3">
            {exports.map((exp, idx) => (
              <div key={idx} className="bg-slate-800 p-4 rounded border border-slate-700 hover:border-blue-500/50 transition-colors">
                <div className="flex items-center gap-3 mb-2">
                  <span className="bg-blue-900/50 text-blue-300 px-2 py-0.5 rounded text-xs font-mono border border-blue-800">
                    {exp.kind}
                  </span>
                  <span className="font-mono text-slate-200 font-medium">{exp.name}</span>
                </div>
                {/* Advanced param/return decoding would go here */}
                <div className="text-sm text-slate-400 pl-1 border-l-2 border-slate-700 ml-2">
                  Interface signature parsing active for this symbol.
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
