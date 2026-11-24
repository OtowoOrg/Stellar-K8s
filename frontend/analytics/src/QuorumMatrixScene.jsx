import { useEffect, useRef } from 'react';
import * as THREE from 'three';
import { buildQuorumMatrix, inspectMatrixCell } from './quorumMatrix.js';

const CELL_SIZE = 0.035;
const GAP = 0.003;
const COLOR_LOW = new THREE.Color(0x202b37);
const COLOR_HIGH = new THREE.Color(0x39d98a);

function colorFor(value, color) {
  color.copy(COLOR_LOW).lerp(COLOR_HIGH, Math.min(1, Math.max(0, value)));
  if (value < 0.2) color.lerp(new THREE.Color(0xf05d5e), 0.35 - value);
}

export default function QuorumMatrixScene({ snapshot, onInspect, selectedCell = null }) {
  const mountRef = useRef(null);
  const snapshotRef = useRef(snapshot);
  const onInspectRef = useRef(onInspect);
  const selectedRef = useRef(selectedCell);
  const stateRef = useRef(null);
  useEffect(() => { snapshotRef.current = snapshot; }, [snapshot]);
  useEffect(() => { onInspectRef.current = onInspect; }, [onInspect]);
  useEffect(() => { selectedRef.current = selectedCell; }, [selectedCell]);

  useEffect(() => {
    const mount = mountRef.current;
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x0b1119);
    const camera = new THREE.OrthographicCamera(-4, 4, 4, -4, 0.1, 20);
    camera.position.z = 8;
    const renderer = new THREE.WebGLRenderer({ antialias: false, powerPreference: 'high-performance' });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    mount.appendChild(renderer.domElement);

    const state = { scene, camera, renderer, matrix: buildQuorumMatrix(snapshotRef.current), mesh: null, frame: 0, raycaster: new THREE.Raycaster(), pointer: new THREE.Vector2() };
    stateRef.current = state;
    const geometry = new THREE.PlaneGeometry(CELL_SIZE, CELL_SIZE);
    const material = new THREE.MeshBasicMaterial({ vertexColors: true });
    const mesh = new THREE.InstancedMesh(geometry, material, 1);
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    mesh.frustumCulled = false;
    state.mesh = mesh;
    scene.add(mesh);

    const resize = () => {
      const width = mount.clientWidth || 800;
      const height = mount.clientHeight || 600;
      renderer.setSize(width, height, false);
      camera.left = -width / height * 4;
      camera.right = width / height * 4;
      camera.top = 4;
      camera.bottom = -4;
      camera.updateProjectionMatrix();
    };
    const pointer = (event) => {
      const rect = renderer.domElement.getBoundingClientRect();
      state.pointer.set(((event.clientX - rect.left) / rect.width) * 2 - 1, -((event.clientY - rect.top) / rect.height) * 2 + 1);
      state.raycaster.setFromCamera(state.pointer, camera);
      const hit = state.raycaster.intersectObject(mesh)[0];
      if (hit?.instanceId !== undefined) {
        const size = state.matrix.size;
        const row = Math.floor(hit.instanceId / size);
        const column = hit.instanceId % size;
        onInspectRef.current?.(inspectMatrixCell(state.matrix, row, column));
      }
    };
    const observer = new ResizeObserver(resize);
    observer.observe(mount);
    renderer.domElement.addEventListener('pointermove', pointer);
    renderer.domElement.addEventListener('pointerdown', pointer);
    resize();

    const temp = new THREE.Object3D();
    const color = new THREE.Color();
    const render = () => {
      state.frame = requestAnimationFrame(render);
      state.matrix = buildQuorumMatrix(snapshotRef.current);
      const size = state.matrix.size;
      if (mesh.count !== size * size) mesh.count = size * size;
      const offset = ((size - 1) * (CELL_SIZE + GAP)) / 2;
      for (let row = 0; row < size; row += 1) for (let column = 0; column < size; column += 1) {
        const index = row * size + column;
        temp.position.set(column * (CELL_SIZE + GAP) - offset, offset - row * (CELL_SIZE + GAP), 0);
        temp.updateMatrix();
        mesh.setMatrixAt(index, temp.matrix);
        colorFor(state.matrix.values[index], color);
        if (selectedRef.current?.row === row && selectedRef.current?.column === column) color.setHex(0xffffff);
        mesh.setColorAt(index, color);
      }
      mesh.instanceMatrix.needsUpdate = true;
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
      renderer.render(scene, camera);
    };
    render();
    return () => {
      cancelAnimationFrame(state.frame);
      observer.disconnect();
      renderer.domElement.removeEventListener('pointermove', pointer);
      renderer.domElement.removeEventListener('pointerdown', pointer);
      geometry.dispose();
      material.dispose();
      renderer.dispose();
      mount.removeChild(renderer.domElement);
      stateRef.current = null;
    };
  }, []);

  return <div className="matrix-scene-host" ref={mountRef} aria-label="Interactive quorum intersection matrix" />;
}
