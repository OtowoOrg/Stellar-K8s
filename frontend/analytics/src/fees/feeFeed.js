import { createFeeState, ingestFeeSample } from './feeModel.js';

const FEED_EVENT = 'fee';
const RECONNECT_MS = 2500;
const listeners = new Set();

let state = { ...createFeeState(), connection: 'idle', source: null };
let socket = null;
let publishFrame = null;
let retryTimer = null;
let disposed = true;

export function getFeeFeedState() {
  return state;
}

export function subscribeFeeFeed(handler) {
  listeners.add(handler);
  handler(state);
  return () => listeners.delete(handler);
}

export function publishFeeFeed() {
  if (publishFrame !== null) return;
  publishFrame = requestAnimationFrame(() => {
    publishFrame = null;
    const snapshot = state;
    for (const handler of listeners) handler(snapshot);
  });
}

export function feeStreamUrl(source) {
  const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
  if (source === 'mock') return `${protocol}://${window.location.hostname}:8788`;
  return `${protocol}://${window.location.host}/api/v1/quorum/topology/stream`;
}

function setConnection(connection) {
  if (disposed) return;
  state = { ...state, connection };
  publishFeeFeed();
}

function handleMessage(event) {
  if (disposed) return;
  try {
    const next = ingestFeeSample(state, JSON.parse(event.data));
    if (next === state) return;
    state = { ...next, connection: state.connection, source: state.source };
    publishFeeFeed();
  } catch {
    setConnection('error');
  }
}

function scheduleReconnect() {
  setConnection('offline');
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = setTimeout(() => {
    retryTimer = null;
    if (!disposed) startFeeFeed(state.source);
  }, RECONNECT_MS);
}

export function startFeeFeed(source = 'mock') {
  stopFeeFeed();
  disposed = false;
  state = { ...createFeeState(), connection: 'connecting', source };
  publishFeeFeed();
  let client;
  try {
    client = new WebSocket(feeStreamUrl(source));
  } catch {
    setConnection('error');
    return;
  }
  socket = client;
  client.onopen = () => setConnection('live');
  client.onmessage = handleMessage;
  client.onerror = () => setConnection('error');
  client.onclose = () => {
    if (disposed) return;
    scheduleReconnect();
  };
}

export function stopFeeFeed() {
  disposed = true;
  if (retryTimer) {
    clearTimeout(retryTimer);
    retryTimer = null;
  }
  if (socket) {
    socket.onclose = null;
    socket.close();
    socket = null;
  }
  if (publishFrame !== null) {
    cancelAnimationFrame(publishFrame);
    publishFrame = null;
  }
}