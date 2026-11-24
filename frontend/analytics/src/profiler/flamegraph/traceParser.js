export const MAX_DEPTH = 20;

export const parseSorobanTrace = (traceData) => {
  if (!traceData) return [];

  const traces = Array.isArray(traceData) ? traceData : [traceData];

  return traces.map((trace) => {
    const result = {
      contractId: trace.id || trace.contractId || 'unknown',
      contractName: trace.contractName || trace.name || 'Unknown Contract',
      gasUsed: trace.gas_used || trace.gasUsed || 0,
      instructionCount: trace.instruction_count || trace.instructionCount || 0,
      subcalls: [],
    };

    if (trace.v && trace.v.subcalls) {
      result.subcalls = parseSorobanTrace(trace.v.subcalls);
    } else if (trace.subcalls) {
      result.subcalls = parseSorobanTrace(trace.subcalls);
    } else if (trace.innerCalls) {
      result.subcalls = parseSorobanTrace(trace.innerCalls);
    }

    return result;
  });
};

export const buildFlamegraphData = (traces) => {
  const parsed = parseSorobanTrace(traces);

  const buildNode = (call, depth = 0) => {
    if (depth > MAX_DEPTH) return null;

    const children = call.subcalls || [];
    const node = {
      id: call.contractId || 'unknown',
      name: call.contractName || call.name || `Contract ${call.id?.slice(0, 8) || '???'}`,
      cost: call.instructionCount || call.gasUsed || 0,
      children: [],
    };

    for (const child of children) {
      const childNode = buildNode(child, depth + 1);
      if (childNode) {
        node.children.push(childNode);
      }
    }

    return node;
  };

  return parsed.map(buildNode).filter((n) => n !== null);
};

export const getTotalInstructionCount = (nodes) => {
  if (!nodes || nodes.length === 0) return 0;

  let total = 0;

  const traverse = (node) => {
    if (!node) return;
    total += node.cost || 0;
    for (const child of node.children) {
      traverse(child);
    }
  };

  for (const node of nodes) {
    traverse(node);
  }

  return total;
};