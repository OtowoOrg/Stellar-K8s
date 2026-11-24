import { useEffect, useRef } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { statusForNode } from './graphModel.js';

const STATUS_COLORS = {
  synced: 0x2ee59d,
  degraded: 0xf7c948,
  'falling-behind': 0xff5c7a,
};
const NODE_RADIUS = 0.16;
const MAX_EDGE_SEGMENTS = 20000;

function hashPosition(id, index) {
  let hash = 2166136261;
  for (let i = 0; i < id.length; i += 1) hash = Math.imul(hash ^ id.charCodeAt(i), 16777619);
  const angle = (Math.abs(hash) % 628) / 100;
  const radius = 2.4 + (index % 19) * 0.16;
  return new THREE.Vector3(
    Math.cos(angle) * radius,
    ((index % 13) - 6) * 0.16,
    Math.sin(angle) * radius,
  );
}

function makeNodeMaterial() {
  return new THREE.MeshBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.96 });
}

export default function TopologyScene({
  graph,
  onSelect,
  selectedId = null,
  paused = false,
  onFrame,
}) {
  const mountRef = useRef(null);
  const graphRef = useRef(graph);
  const pausedRef = useRef(paused);
  const onSelectRef = useRef(onSelect);
  const onFrameRef = useRef(onFrame);
  const sceneState = useRef(null);

  useEffect(() => {
    graphRef.current = graph;
  }, [graph]);
  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);
  useEffect(() => {
    onSelectRef.current = onSelect;
  }, [onSelect]);
  useEffect(() => {
    onFrameRef.current = onFrame;
  }, [onFrame]);
  useEffect(() => {
    if (sceneState.current) sceneState.current.selected = selectedId;
  }, [selectedId]);

  useEffect(() => {
    const mount = mountRef.current;
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x05070a);
    scene.fog = new THREE.FogExp2(0x05070a, 0.028);

    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.set(0, 0, 11);

    const renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance' });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.75));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.domElement.tabIndex = 0;
    renderer.domElement.setAttribute('aria-label', 'Interactive 3D Stellar validator topology');
    mount.appendChild(renderer.domElement);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.minDistance = 2.5;
    controls.maxDistance = 30;
    controls.target.set(0, 0, 0);

    scene.add(new THREE.AmbientLight(0xffffff, 1.5));
    const halo = new THREE.PointLight(0x5bd8ff, 18, 30);
    halo.position.set(-3, 2, 5);
    scene.add(halo);

    const state = {
      scene,
      camera,
      renderer,
      controls,
      nodes: new THREE.InstancedMesh(new THREE.SphereGeometry(NODE_RADIUS, 8, 6), makeNodeMaterial(), 1),
      edges: new THREE.LineSegments(
        new THREE.BufferGeometry(),
        new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.42 }),
      ),
      nodeCapacity: 1,
      positions: new Map(),
      velocities: new Map(),
      forces: new Map(),
      selected: null,
      raycaster: new THREE.Raycaster(),
      pointer: new THREE.Vector2(),
      frame: 0,
      lastSimulation: 0,
      frameCount: 0,
      frameWindowStart: performance.now(),
      onFrameRef,
    };
    state.nodes.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    state.nodes.frustumCulled = false;
    state.edges.frustumCulled = false;
    scene.add(state.nodes, state.edges);
    sceneState.current = state;

    const resize = () => {
      const width = mount.clientWidth || 800;
      const height = mount.clientHeight || 600;
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
    };

    const selectFromPointer = (event) => {
      const rect = renderer.domElement.getBoundingClientRect();
      state.pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
      state.pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
      state.raycaster.setFromCamera(state.pointer, camera);
      const hit = state.raycaster.intersectObject(state.nodes)[0];
      if (!hit) return;
      const node = graphRef.current.nodes[hit.instanceId];
      if (node) {
        state.selected = node.id;
        onSelectRef.current(node);
      }
    };

    const observer = new ResizeObserver(resize);
    observer.observe(mount);
    renderer.domElement.addEventListener('pointerdown', selectFromPointer);
    resize();

    const temp = new THREE.Object3D();
    const color = new THREE.Color();
    const animate = (time) => {
      state.frame = requestAnimationFrame(animate);
      const current = graphRef.current;
      if (!pausedRef.current && time - state.lastSimulation > 45) {
        state.lastSimulation = time;
        simulate(current, state, time);
      }
      updateInstances(current, state, temp, color);
      controls.update();
      renderer.render(scene, camera);
      publishFrameStats(state, time, current);
    };
    state.frame = requestAnimationFrame(animate);

    return () => {
      cancelAnimationFrame(state.frame);
      observer.disconnect();
      renderer.domElement.removeEventListener('pointerdown', selectFromPointer);
      controls.dispose();
      state.nodes.geometry.dispose();
      state.nodes.material.dispose();
      state.edges.geometry.dispose();
      state.edges.material.dispose();
      renderer.dispose();
      mount.removeChild(renderer.domElement);
      sceneState.current = null;
    };
  }, []);

  useEffect(() => {
    const state = sceneState.current;
    if (!state) return;
    const ids = new Set(graph.nodes.map((node) => node.id));
    for (const id of state.positions.keys()) {
      if (!ids.has(id)) {
        state.positions.delete(id);
        state.velocities.delete(id);
        state.forces.delete(id);
      }
    }
    graph.nodes.forEach((node, index) => {
      if (!state.positions.has(node.id)) {
        state.positions.set(node.id, hashPosition(node.id, index));
        state.velocities.set(node.id, new THREE.Vector3());
        state.forces.set(node.id, new THREE.Vector3());
      }
    });
  }, [graph.nodes]);

  return <div className="scene-host" ref={mountRef} aria-label="Interactive 3D network topology" />;
}

function simulate(graph, state, time) {
  const nodes = graph.nodes;
  const { positions, velocities, forces } = state;
  for (const node of nodes) {
    const force = forces.get(node.id);
    if (force) force.set(0, 0, 0);
  }

  for (const edge of graph.edges) {
    const source = positions.get(edge.source);
    const target = positions.get(edge.target);
    if (!source || !target) continue;
    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const dz = target.z - source.z;
    const distance = Math.max(Math.sqrt(dx * dx + dy * dy + dz * dz), 0.05);
    const scale = (distance - 1.15) * 0.003 / distance;
    const sourceForce = forces.get(edge.source);
    const targetForce = forces.get(edge.target);
    if (sourceForce && targetForce) {
      sourceForce.x += dx * scale;
      sourceForce.y += dy * scale;
      sourceForce.z += dz * scale;
      targetForce.x -= dx * scale;
      targetForce.y -= dy * scale;
      targetForce.z -= dz * scale;
    }
  }

  const sampleStride = nodes.length > 800 ? Math.ceil(nodes.length / 120) : nodes.length > 500 ? 2 : 1;
  for (let i = 0; i < nodes.length; i += 1) {
    const node = nodes[i];
    const position = positions.get(node.id);
    const force = forces.get(node.id);
    if (!position || !force) continue;
    for (let j = (i + 1) % sampleStride; j < nodes.length; j += sampleStride) {
      if (i === j) continue;
      const other = positions.get(nodes[j].id);
      if (!other) continue;
      const dx = position.x - other.x;
      const dy = position.y - other.y;
      const dz = position.z - other.z;
      const distanceSquared = Math.max(dx * dx + dy * dy + dz * dz, 0.08);
      const scale = 0.0008 / distanceSquared;
      force.x += dx * scale;
      force.y += dy * scale;
      force.z += dz * scale;
    }
    force.x -= position.x * 0.0007;
    force.y -= position.y * 0.0007;
    force.z -= position.z * 0.0007;
    const velocity = velocities.get(node.id);
    if (!velocity) continue;
    velocity.x = (velocity.x + force.x) * 0.91;
    velocity.y = (velocity.y + force.y) * 0.91;
    velocity.z = (velocity.z + force.z) * 0.91;
    position.add(velocity);
    position.y += Math.sin(time * 0.0005 + i) * 0.0005;
  }
}

function updateInstances(graph, state, temp, color) {
  const count = Math.max(graph.nodes.length, 1);
  if (state.nodeCapacity !== count) {
    const old = state.nodes;
    const replacement = new THREE.InstancedMesh(new THREE.SphereGeometry(NODE_RADIUS, 8, 6), makeNodeMaterial(), count);
    replacement.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    replacement.frustumCulled = false;
    state.scene.remove(old);
    old.geometry.dispose();
    old.material.dispose();
    state.nodes = replacement;
    state.nodeCapacity = count;
    state.scene.add(replacement);
  }

  state.nodes.count = graph.nodes.length;
  graph.nodes.forEach((node, index) => {
    const position = state.positions.get(node.id) ?? new THREE.Vector3();
    temp.position.copy(position);
    temp.scale.setScalar(node.id === state.selected ? 1.65 : 1);
    temp.updateMatrix();
    state.nodes.setMatrixAt(index, temp.matrix);
    color.setHex(STATUS_COLORS[statusForNode(node)] ?? STATUS_COLORS.degraded);
    state.nodes.setColorAt(index, color);
  });
  state.nodes.instanceMatrix.needsUpdate = true;
  if (state.nodes.instanceColor) state.nodes.instanceColor.needsUpdate = true;

  let positionAttribute = state.edges.geometry.getAttribute('position');
  let colorAttribute = state.edges.geometry.getAttribute('color');
  if (!positionAttribute || positionAttribute.array.length < MAX_EDGE_SEGMENTS * 6) {
    positionAttribute = new THREE.BufferAttribute(new Float32Array(MAX_EDGE_SEGMENTS * 6), 3);
    colorAttribute = new THREE.BufferAttribute(new Float32Array(MAX_EDGE_SEGMENTS * 6), 3);
    positionAttribute.setUsage(THREE.DynamicDrawUsage);
    colorAttribute.setUsage(THREE.DynamicDrawUsage);
    state.edges.geometry.setAttribute('position', positionAttribute);
    state.edges.geometry.setAttribute('color', colorAttribute);
  }
  const edgeColor = color.setHex(0x526a80);
  graph.edges.forEach((edge, index) => {
    const source = state.positions.get(edge.source) ?? new THREE.Vector3();
    const target = state.positions.get(edge.target) ?? new THREE.Vector3();
    positionAttribute.setXYZ(index * 2, source.x, source.y, source.z);
    positionAttribute.setXYZ(index * 2 + 1, target.x, target.y, target.z);
    colorAttribute.setXYZ(index * 2, edgeColor.r, edgeColor.g, edgeColor.b);
    colorAttribute.setXYZ(index * 2 + 1, edgeColor.r, edgeColor.g, edgeColor.b);
  });
  positionAttribute.needsUpdate = true;
  colorAttribute.needsUpdate = true;
  state.edges.geometry.setDrawRange(0, graph.edges.length * 2);
}

function publishFrameStats(state, time, graph) {
  state.frameCount += 1;
  if (time - state.frameWindowStart < 1000) return;
  const fps = Math.round((state.frameCount * 1000) / (time - state.frameWindowStart));
  state.frameWindowStart = time;
  state.frameCount = 0;
  const memory = performance.memory?.usedJSHeapSize
    ? Math.round(performance.memory.usedJSHeapSize / 1024 / 1024)
    : null;
  onFrameStats(state, { fps, memory, nodes: graph.nodes.length, edges: graph.edges.length });
}

function onFrameStats(state, stats) {
  state.onFrameRef.current?.(stats);
}
