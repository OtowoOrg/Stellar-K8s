export const MATRIX_MAX_NODES = 200;

const clamp = (value, min = 0, max = 1) => Math.min(max, Math.max(min, Number.isFinite(Number(value)) ? Number(value) : min));

function aliases(value) {
  const id = String(value?.id ?? value?.node_id ?? value?.nodeId ?? value?.full_id ?? value?.fullId ?? value ?? '');
  return id ? [id, id.length > 12 ? `${id.slice(0, 4)}...${id.slice(-4)}` : id, id.length > 12 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id] : [];
}

function quorumMembers(node) {
  const quorum = node?.quorum_set ?? node?.quorumSet ?? node?.quorum ?? {};
  return [
    ...(quorum.validators ?? quorum.v ?? []),
    ...(quorum.inner_sets ?? quorum.innerSets ?? []).flatMap((set) => set.validators ?? set.v ?? []),
  ].map((member) => String(member?.id ?? member?.node_id ?? member?.nodeId ?? member?.full_id ?? member?.fullId ?? member));
}

export function normalizeMatrixNodes(snapshot = {}) {
  const source = Array.isArray(snapshot.nodes) ? snapshot.nodes : [];
  return source.slice(0, MATRIX_MAX_NODES).map((node, index) => ({
    id: String(node.id ?? node.node_id ?? node.nodeId ?? node.full_id ?? node.fullId ?? `validator-${index + 1}`),
    fullId: String(node.full_id ?? node.fullId ?? node.node_id ?? node.nodeId ?? node.id ?? ''),
    name: String(node.node_name ?? node.nodeName ?? node.organization ?? node.org ?? node.name ?? `Validator ${index + 1}`),
    organization: String(node.organization ?? node.org ?? node.cluster ?? node.namespace ?? 'Unknown organization'),
    cluster: String(node.cluster ?? node.namespace ?? 'default'),
    trust: clamp(node.trust_percentage ?? node.trustPercentage ?? node.agreement ?? node.agreement_percentage ?? node.trust ?? 0),
    quorumSet: quorumMembers(node),
  }));
}

export function buildQuorumMatrix(snapshot = {}) {
  const nodes = normalizeMatrixNodes(snapshot);
  const identity = new Map();
  nodes.forEach((node) => aliases(node.id).concat(aliases(node.fullId)).forEach((alias) => identity.set(alias, node.id)));
  const sets = new Map(nodes.map((node) => [node.id, new Set(node.quorumSet.map((member) => identity.get(member) ?? member))]));
  const values = new Float32Array(nodes.length * nodes.length);
  const overlaps = new Uint16Array(values.length);

  for (let row = 0; row < nodes.length; row += 1) {
    const left = sets.get(nodes[row].id) ?? new Set();
    for (let column = 0; column < nodes.length; column += 1) {
      const right = sets.get(nodes[column].id) ?? new Set();
      let common = 0;
      left.forEach((member) => { if (right.has(member)) common += 1; });
      const union = new Set([...left, ...right]).size;
      const overlap = union ? common / union : row === column ? 1 : 0;
      const agreement = row === column ? 1 : (nodes[row].trust + nodes[column].trust) / 2;
      const index = row * nodes.length + column;
      values[index] = clamp(agreement) * overlap;
      overlaps[index] = common;
    }
  }
  return { nodes, values, overlaps, size: nodes.length };
}

export function inspectMatrixCell(matrix, row, column) {
  if (!matrix || row < 0 || column < 0 || row >= matrix.size || column >= matrix.size) return null;
  const index = row * matrix.size + column;
  const source = matrix.nodes[row];
  const target = matrix.nodes[column];
  const common = [...new Set(source.quorumSet)].filter((member) => new Set(target.quorumSet).has(member));
  return {
    row,
    column,
    source,
    target,
    agreement: matrix.values[index],
    commonDependencies: common,
    overlapCount: matrix.overlaps[index],
  };
}
