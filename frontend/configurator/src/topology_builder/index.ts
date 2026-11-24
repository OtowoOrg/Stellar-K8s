/**
 * Barrel export for the topology_builder module.
 *
 * Re-exports all public types, state management utilities, validation logic,
 * and React components from the topology builder sub-packages, providing a
 * single convenient import point for consumers.
 *
 * @example
 * ```ts
 * import {
 *   TopologyProvider,
 *   useTopology,
 *   validateTopology,
 *   TopologyBuilder,
 * } from './topology_builder';
 * ```
 */

// Core domain types
export * from './types';

// State management — reducer, context, provider, hooks, and initial state
export * from './topology_store';

// Quorum validation engine
export * from './quorum_validator';

// React components
export { default as WorkerNode } from './WorkerNode';
export { default as AvailabilityZone } from './AvailabilityZone';
export { default as StellarNodePlacer } from './StellarNodePlacer';
export { default as TopologyBuilder } from './TopologyBuilder';
