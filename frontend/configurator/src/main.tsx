/**
 * Application entry point for the Stellar-K8s Topology Configurator.
 *
 * Bootstraps the React application by mounting the root component tree into
 * the DOM element with id "root". The full component tree is wrapped in
 * React.StrictMode to surface potential issues during development, and in
 * TopologyProvider to make topology state available throughout the tree.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { TopologyProvider, TopologyBuilder } from './topology_builder';
import { createInitialState } from './topology_builder/topology_store';

import './styles.css';

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error(
    '[Stellar-K8s Topology Configurator] Root element with id "root" not found in the document. ' +
      'Ensure your index.html contains <div id="root"></div>.',
  );
}

createRoot(rootElement).render(
  <StrictMode>
    <TopologyProvider initialState={createInitialState()}>
      <TopologyBuilder />
    </TopologyProvider>
  </StrictMode>,
);
