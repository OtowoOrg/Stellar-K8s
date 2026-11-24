#!/usr/bin/env node
import { Kafka, logLevel } from 'kafkajs';
import { WebSocketServer } from 'ws';

const brokers = (process.env.KAFKA_BROKERS ?? 'localhost:9092').split(',').map((value) => value.trim()).filter(Boolean);
const topic = process.env.KAFKA_TOPIC ?? 'stellar-scp-messages';
const groupId = process.env.KAFKA_GROUP_ID ?? 'stellar-analytics-visualizer';
const port = Number(process.env.ANALYTICS_WS_PORT ?? 8787);
const kafka = new Kafka({ clientId: 'stellar-analytics-visualizer', brokers, logLevel: logLevel.NOTHING });
const consumer = kafka.consumer({ groupId });
const server = new WebSocketServer({ port });
const clients = new Set();

server.on('connection', (socket) => {
  clients.add(socket);
  socket.on('close', () => clients.delete(socket));
});

function broadcast(value) {
  const payload = JSON.stringify(value);
  for (const socket of clients) {
    if (socket.readyState === 1) socket.send(payload);
  }
}

async function start() {
  await consumer.connect();
  await consumer.subscribe({ topic, fromBeginning: process.env.KAFKA_FROM_BEGINNING === 'true' });
  await consumer.run({
    eachMessage: async ({ message }) => {
      if (!message.value) return;
      try {
        broadcast(JSON.parse(message.value.toString('utf8')));
      } catch {
        console.warn('Skipping non-JSON Kafka message');
      }
    },
  });
  console.log(`Kafka bridge listening on ws://localhost:${port}`);
  console.log(`Topic: ${topic}; brokers: ${brokers.join(', ')}`);
}

async function shutdown() {
  server.close();
  await consumer.disconnect();
  process.exit(0);
}

process.once('SIGINT', shutdown);
process.once('SIGTERM', shutdown);
start().catch(async (error) => {
  console.error(`Kafka bridge failed: ${error.message}`);
  await consumer.disconnect().catch(() => {});
  process.exit(1);
});
