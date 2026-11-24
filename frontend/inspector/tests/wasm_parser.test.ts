import { parseWasmExports } from '../../services/wasm_parser';

describe('WASM Parser', () => {
  it('should parse exported functions from standard WASM bytecode', async () => {
    // A minimal valid WebAssembly module that exports a function named 'hello_world'
    // (module (func (export "hello_world")))
    const minimalWasm = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // Magic & Version
      0x01, 0x04, 0x01, 0x60, 0x00, 0x00,             // Type section: 1 type, func()
      0x03, 0x02, 0x01, 0x00,                         // Function section: 1 func, type index 0
      0x07, 0x0f, 0x01, 0x0b, 0x68, 0x65, 0x6c, 0x6c, // Export section: 'hello_world'
      0x6f, 0x5f, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x00, 
      0x00, 
      0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b              // Code section: 1 func, empty body
    ]);

    const exports = await parseWasmExports(minimalWasm.buffer);
    
    expect(exports).toBeDefined();
    expect(exports.length).toBeGreaterThan(0);
    expect(exports.some(e => e.name === 'hello_world')).toBe(true);
    expect(exports[0].kind).toBe('function');
  });
});
