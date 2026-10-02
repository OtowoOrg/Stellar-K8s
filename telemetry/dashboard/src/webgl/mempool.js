// Placeholder WebGL mempool visualizer using Three.js
// This file sets up a basic Three.js scene and defines a function to update
// instanced meshes representing mempool transactions.

import * as THREE from 'three';

// Global objects (in a real app these would be managed by the UI framework)
let scene, camera, renderer, instancedMesh;

export function initMempoolVisualizer(container) {
  // Create renderer
  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setSize(container.clientWidth, container.clientHeight);
  container.appendChild(renderer.domElement);

  // Scene and camera
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(
    75,
    container.clientWidth / container.clientHeight,
    0.1,
    1000
  );
  camera.position.z = 50;

  // Geometry for a transaction node (a small box)
  const geometry = new THREE.BoxGeometry(1, 1, 1);
  const material = new THREE.MeshBasicMaterial({ vertexColors: true });

  // Instanced mesh – allocate enough instances for up to 20,000 txs
  const maxInstances = 20000;
  instancedMesh = new THREE.InstancedMesh(geometry, material, maxInstances);
  scene.add(instancedMesh);

  // Initialize instance matrices and colors
  const dummy = new THREE.Object3D();
  for (let i = 0; i < maxInstances; i++) {
    dummy.position.set(0, 0, 0);
    dummy.updateMatrix();
    instancedMesh.setMatrixAt(i, dummy.matrix);
    const color = new THREE.Color(0x00ff00);
    instancedMesh.setColorAt(i, color);
  }
  instancedMesh.instanceMatrix.needsUpdate = true;
  if (instancedMesh.instanceColor) instancedMesh.instanceColor.needsUpdate = true;

  animate();
}

// Update the visualizer with an array of tx objects:
// [{ size_bytes: number, fee_bid: number, ... }, ...]
export function updateMempool(txs) {
  const dummy = new THREE.Object3D();
  const max = Math.min(txs.length, instancedMesh.count);
  for (let i = 0; i < max; i++) {
    const tx = txs[i];
    // Position nodes in a simple grid for demo purposes
    const x = (i % 100) * 1.5 - 75;
    const y = Math.floor(i / 100) * 1.5 - 75;
    dummy.position.set(x, y, 0);
    dummy.scale.set(Math.cbrt(tx.size_bytes), Math.cbrt(tx.size_bytes), Math.cbrt(tx.size_bytes));
    dummy.updateMatrix();
    instancedMesh.setMatrixAt(i, dummy.matrix);
    // Color based on fee bid (higher fee → greener)
    const feeNorm = Math.min(Math.max((tx.fee_bid || 0) / 10000, 0), 1);
    const color = new THREE.Color().setRGB(1 - feeNorm, feeNorm, 0);
    instancedMesh.setColorAt(i, color);
  }
  instancedMesh.instanceMatrix.needsUpdate = true;
  if (instancedMesh.instanceColor) instancedMesh.instanceColor.needsUpdate = true;
}

function animate() {
  requestAnimationFrame(animate);
  renderer.render(scene, camera);
}
