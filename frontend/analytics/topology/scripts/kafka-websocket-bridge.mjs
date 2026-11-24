#!/usr/bin/env node
import { Kafka } from 'kafkajs';
import { WebSocketServer } from 'ws';

const brokers = (process.env.KAFKA_BROKERS ?? 'localhost:9092').split(',').map((broker) => broker.trim()).filter(Boolean);
const topic = process.env.KAFKA_TOPIC ?? 'stellar-scp-messages';
const groupId = process.env.KAFKA_GROUP_ID ?? 'stellar-k8s-topology-visualizer';
const port = Number(process.env.TOPOLOGY_WS_PORT ?? 8787);
const fromBeginning = process.env.KAFKA_FROM_BEGINNING === 'true';

const sockets = new Set();
const server = new WebSocketServer({ port });
server.on('listening', () => {
  console.log(`Kafka bridge listening on ws://localhost:${port}`);
  console.log(`Consuming ${topic} from ${brokers.join(', ')}`);
});
server.on('connection', (socket) => {
  sockets.add(socket);
  socket.on('close', () => sockets.delete(socket));
});

function broadcast(value) {
  for (const socket of sockets) {
    if (socket.readyState === socket.OPEN) socket.send(value);
  }
}

const kafka = new Kafka({ clientId: 'stellar-k8s-topology-visualizer', brokers });
const consumer = kafka.consumer({ groupId });

await consumer.connect();
await consumer.subscribe({ topic, fromBeginning });
await consumer.run({
  eachMessage: async ({ message }) => {
    const value = message.value?.toString();
    if (!value) return;
    try {
      JSON.parse(value);
      broadcast(value);
    } catch {
      console.warn('Skipping non-JSON Kafka message');
    }
  },
});

const shutdown = async () => {
  await consumer.disconnect();
  server.close();
  process.exit(0);
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
