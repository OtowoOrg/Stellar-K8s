import test from 'node:test';
import assert from 'node:assert/strict';
import { parseSorobanTrace, buildFlamegraphData, getTotalInstructionCount } from './traceParser.js';

test('parseSorobanTrace - parses a simple trace object', () => {
  const trace = {
    id: 'contract_123',
    contractName: 'Swap',
    gas_used: 150000,
    instruction_count: 45000,
  };

  const result = parseSorobanTrace(trace);
  assert.equal(result.length, 1);
  assert.equal(result[0].contractId, 'contract_123');
  assert.equal(result[0].contractName, 'Swap');
  assert.equal(result[0].gasUsed, 150000);
  assert.equal(result[0].instructionCount, 45000);
  assert.equal(result[0].subcalls.length, 0);
});

test('parseSorobanTrace - parses nested subcalls', () => {
  const trace = {
    id: 'contract_123',
    contractName: 'Swap',
    gas_used: 150000,
    instruction_count: 45000,
    subcalls: [
      {
        id: 'contract_456',
        contractName: 'SwapStep',
        gas_used: 50000,
        instruction_count: 15000,
      },
    ],
  };

  const result = parseSorobanTrace(trace);
  assert.equal(result.length, 1);
  assert.equal(result[0].subcalls.length, 1);
  assert.equal(result[0].subcalls[0].contractName, 'SwapStep');
  assert.equal(result[0].subcalls[0].instructionCount, 15000);
});

test('parseSorobanTrace - handles undefined trace', () => {
  const result = parseSorobanTrace(undefined);
  assert.equal(result.length, 0);

  const result2 = parseSorobanTrace(null);
  assert.equal(result2.length, 0);
});

test('buildFlamegraphData - builds flamegraph data from traces', () => {
  const traces = {
    id: 'contract_123',
    contractName: 'Swap',
    gas_used: 150000,
    instruction_count: 45000,
    subcalls: [
      {
        id: 'contract_456',
        contractName: 'SwapStep',
        gas_used: 50000,
        instruction_count: 15000,
        subcalls: [
          {
            id: 'contract_789',
            contractName: 'InnerCall',
            gas_used: 20000,
            instruction_count: 5000,
          },
        ],
      },
    ],
  };

  const result = buildFlamegraphData(traces);
  assert.equal(result.length, 1);
  assert.equal(result[0].name, 'Swap');
  assert.equal(result[0].cost, 45000);
  assert.equal(result[0].children.length, 1);
  assert.equal(result[0].children[0].name, 'SwapStep');
  assert.equal(result[0].children[0].cost, 15000);
  assert.equal(result[0].children[0].children.length, 1);
  assert.equal(result[0].children[0].children[0].name, 'InnerCall');
  assert.equal(result[0].children[0].children[0].cost, 5000);
});

test('buildFlamegraphData - handles empty traces', () => {
  const result = buildFlamegraphData([]);
  assert.equal(result.length, 0);
});

test('buildFlamegraphData - handles traces with no subcalls', () => {
  const traces = {
    id: 'contract_123',
    contractName: 'Simple',
    gas_used: 10000,
    instruction_count: 3000,
  };

  const result = buildFlamegraphData(traces);
  assert.equal(result.length, 1);
  assert.equal(result[0].children.length, 0);
  assert.equal(result[0].cost, 3000);
});

test('getTotalInstructionCount - calculates total from nested nodes', () => {
  const nodes = [
    {
      id: '1',
      name: 'Level1',
      cost: 100,
      children: [
        {
          id: '1.1',
          name: 'Level2',
          cost: 50,
          children: [
            {
              id: '1.1.1',
              name: 'Level3',
              cost: 25,
              children: [],
            },
          ],
        },
      ],
    },
  ];

  const result = getTotalInstructionCount(nodes);
  assert.equal(result, 175);
});

test('getTotalInstructionCount - returns 0 for empty nodes', () => {
  const result = getTotalInstructionCount([]);
  assert.equal(result, 0);
});

test('getTotalInstructionCount - returns 0 for null nodes', () => {
  const result = getTotalInstructionCount([null]);
  assert.equal(result, 0);
});