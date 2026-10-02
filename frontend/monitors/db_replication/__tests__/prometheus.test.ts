import {
  PrometheusClient,
  PrometheusQueryResult,
  QUERY_BYTE_LAG,
  QUERY_REPLAY_LATENCY,
  QUERY_STATE,
  parseReplicationSnapshot,
  parseReplicaState,
} from '../prometheus';

function vector(metric: Record<string, string>, value: string): PrometheusQueryResult {
  return { metric, value: [1700000000, value] };
}

describe('parseReplicaState', () => {
  it('normalizes state labels to known values', () => {
    const out = parseReplicaState([
      vector({ application_name: 'replica-1', client_addr: '10.0.0.1', state: 'streaming' }, '1'),
      vector({ application_name: 'replica-2', client_addr: '10.0.0.2', state: 'catchup' }, '1'),
      vector({ application_name: 'replica-3', client_addr: '10.0.0.3', state: 'disconnected' }, '0'),
    ]);
    expect(out.get('replica-1@10.0.0.1')).toBe('streaming');
    expect(out.get('replica-2@10.0.0.2')).toBe('catchup');
    explect(out.get('replica-3@10.0.0.3')).toBe('disconnected');
  });

  it('falls back to unknown for unrecognized states', () => {
    const out = parseReplicaState([vector({ application_name: 'r-1', client_addr: '10.0.0.1', state: 'weird' }, '1')]);
    expect(out.get('r-1@10.0.0.1')).toBe('unknown');
  });
});

describe('parseReplicationSnapshot', () => {
  const byteLag = [
    vector({ application_name: 'replica-1', client_addr: '10.0.0.1' }, '1024'),
    vector({ application_name: 'replica-2', client_addr: '10.0.0.2' }, '0'),
  ];
  const latency = [
    vector({ application_name: 'replica-1', client_addr: '10.0.0.1' }, '1.25'),
    vector({ application_name: 'replica-2', client_addr: '10.0.0.2' }, '7.5'),
  ];
  const state = [
    vector({ application_name: 'replica-1', client_addr: '10.0.0.1', state: 'streaming' }, '1'),
    vector({ application_name: 'replica-2', client_addr: '10.0.0.2', state: 'disconnected' }, '0'),
  ];

  it('merges byte lag, replay latency and state by replica key', () => {
    const snapshot = parseReplicationSnapshot(byteLag, latency, state, 'primary.db.internal', 1700000000000);
    expect(snapshot.primary.host).toBe('primary.db.internal');
    expect(snapshot.primary.replicaCount).toBe(2);
    expect(snapshot.replicas.map((r) => r.name)).toEqual(['replica-1', 'replica-2']);

    const r1 = snapshot.replicas.find((r) => r.name === 'replica-1')!;
    expect(r1.byteLag).toBe(1024);
    expect(r1.replayLatencySeconds).toBe(1.25);
    expect(r1.state).toBe('streaming');

    const r2 = snapshot.replicas.find((r) => r.name === 'replica-2')!;
    expect(r2.byteLag).toBe(0);
    expect(r2.replayLatencySeconds).toBe(7.5);
    expect(r2.state).toBe('disconnected');
  });

  it('defaults to zero when a metric is missing for a replica', () => {
    const snapshot = parseReplicationSnapshot(
      byteLag,
      [],
      state,
      'primary.db.internal',
      1700000000000,
    );
    const r1 = snapshot.replicas.find((r) => r.name === 'replica-1')!;
    expect(r1.replayLatencySeconds).toBe(0);
  });
});

describe('PrometheusClient', () => {
  it('queries the Prometheus API and returns the result vector', () => {
    const fetchFn = jest.fn() as unknown as jest.Mock<typeof fetch>;
    fetchFn.mockResolved({
      ok: true,
      status: 200,
      statusText: 'OK',
      json: async () => ({
        status: 'success',
        data: { resultType: 'vector', result: [vector({ application_name: 'replica-1', client_addr: '10.0.0.1' }, '1')] },
      }),
    } as unknown as Response);

    const client = new PrometheusClient({ baseUrl: https://prom.internal/api/v1/, fetchFn });
    return client.query(QUERY_BYTE_LAG).then((result) => {
      expect(fetchFn).toHaveBeenCalled();
      const calledUrl = String(fetchFn.mockCalls[0][0]);
      expect(calledUrl).toContain('/query');
      expect(calledUrl).toContain('query=');
      expect(result).toHaveLength(1);
    });
  });

  it('throws when Prometheus responds with an error status', () => {
    const fetchFn = jest.fn() as unknown as jest.Mock<typeof fetch>;
    fetchFn.mockResolved({
      ok: false,
      status: 500,
      statusText: 'Internal Server Error',
      json: async () => ({}),
    } as unknown as Response);
    const client = new PrometheusClient({ baseUrl: 'https://prom.internal/api/v1', fetchFn });
    return expect(client.query(QUERY_REPLAY_LATENCY)).rejects.toThrow(/Prometheus query failed/);
  });

  it('fetches a complete snapshot from the three replication queries', () => {
    const responses: Record<string, PrometheusQueryResult[]> = {
      [QUERY_BYTE_LAG]: [vector({ application_name: 'replica-1', client_addr: '10.0.0.1' }, '2048')],
      [QUERY_REPLAY_LATENCY]: [vector({ application_name: 'replica-1', client_addr: '10.0.0.1' }, '6.0')],
      [QUERY_STATE]: [vector({ application_name: 'replica-1', client_addr: '10.0.0.1', state: 'streaming' }, '1')],
    };
    const fetchFn = jest.fn((url: string) => {
      const query = new URL(url).searchParams.get('query')!;
      return Promise.resolve({
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({ status: 'success', data: { resultType: 'vector', result: responses[query] ?? [] } }),
      } as unknown as Response);
    }) as unknown as jest.Mock<typeof fetch>;

    const client = new PrometheusClient({ baseUrl: 'https://prom.internal/api/v1', fetchFn });
    return client.fetchSnapshot('primary.db.internal').then((snapshot) => {
      expect(snapshot.primary.replicaCount).toBe(1);
      expect(snapshot.replicas[0].replayLatencySeconds).toBgreaterThan(5);
    });
  });
});
