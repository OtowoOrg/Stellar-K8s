import * as THREE from 'three';
import { cellShade } from '../../analytics/matrix/quorumMatrixModel.js';

export const MAX_INSTANCED_CELLS = 65535;

const VERTEX_SHADER = /* glsl */ `
  attribute vec3 cellColor;
  attribute float cellOpacity;
  varying vec3 vColor;
  varying float vOpacity;

  void main() {
    vColor = cellColor;
    vOpacity = cellOpacity;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

const FRAGMENT_SHADER = /* glsl */ `
  varying vec3 vColor;
  varying float vOpacity;

  void main() {
    gl_FragColor = vec4(vColor, vOpacity);
  }
`;

function makeGeometry(capacity) {
  const geometry = new THREE.InstancedBufferGeometry();
  geometry.setIndex([0, 1, 2, 2, 1, 3]);
  geometry.setAttribute('position', new THREE.BufferAttribute(new Float32Array([
    -0.5, -0.5, 0,
    0.5, -0.5, 0,
    -0.5, 0.5, 0,
    0.5, 0.5, 0,
  ]), 3));
  const instanceMatrix = new THREE.InstancedBufferAttribute(new Float32Array(capacity * 16), 16);
  instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute('instanceMatrix', instanceMatrix);
  const cellColor = new THREE.InstancedBufferAttribute(new Float32Array(capacity * 3), 3);
  cellColor.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute('cellColor', cellColor);
  const cellOpacity = new THREE.InstancedBufferAttribute(new Float32Array(capacity), 1);
  cellOpacity.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute('cellOpacity', cellOpacity);
  geometry.instanceCount = 0;
  geometry.boundingSphere = new THREE.Sphere(new THREE.Vector3(), Infinity);
  return geometry;
}

export class QuorumMatrixRenderer {
  constructor({ canvas, cellSize = 1, gap = 0.08 } = {}) {
    this.canvas = canvas;
    this.cellSize = cellSize;
    this.gap = gap;
    this.cellCount = 0;
    this.size = 0;
    this.highlight = null;

    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: false, powerPreference: 'high-performance' });
    this.renderer.setPixelRatio(1);
    this.renderer.setClearColor(0x0b1119, 1);
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;

    this.scene = new THREE.Scene();
    this.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 100);
    this.camera.position.set(0, 0, 10);

    this.geometry = makeGeometry(MAX_INSTANCED_CELLS);
    this.material = new THREE.ShaderMaterial({
      vertexShader: VERTEX_SHADER,
      fragmentShader: FRAGMENT_SHADER,
      transparent: true,
      depthTest: false,
      depthWrite: false,
    });
    this.mesh = new THREE.Mesh(this.geometry, this.material);
    this.mesh.frustumCulled = false;
    this.scene.add(this.mesh);
  }

  resize(width, height) {
    this.renderer.setSize(width, height, false);
    const aspect = width / Math.max(height, 1);
    const half = (this.size * (this.cellSize + this.gap)) / 2 + this.cellSize;
    this.camera.left = -half * aspect;
    this.camera.right = half * aspect;
    this.camera.top = half;
    this.camera.bottom = -half;
    this.camera.updateProjectionMatrix();
  }

  // Uploads instance buffers and redraws only when the matrix or highlight
  // changes, so an idle canvas costs nothing on the UI thread or GPU.
  render(matrix) {
    const highlightKey = this.highlight ? `${this.highlight.sourceIndex}:${this.highlight.targetIndex}` : '';
    const cacheKey = `${matrix.cells.length}:${matrix.size}:${highlightKey}`;
    if (this.renderedKey === cacheKey) {
      this.renderer.render(this.scene, this.camera);
      return;
    }
    this.renderedKey = cacheKey;
    this.size = matrix.size;
    const colorAttr = this.geometry.getAttribute('cellColor');
    const opacityAttr = this.geometry.getAttribute('cellOpacity');
    const instanceMatrix = this.geometry.getAttribute('instanceMatrix');
    const stride = this.cellSize + this.gap;
    const count = Math.min(matrix.cells.length, MAX_INSTANCED_CELLS);
    const half = (this.size - 1) / 2;

    for (let index = 0; index < count; index += 1) {
      const cell = matrix.cells[index];
      const x = (cell.targetIndex - half) * stride;
      const y = (half - cell.sourceIndex) * stride;

      const offset = index * 16;
      instanceMatrix.array.fill(0, offset, offset + 16);
      instanceMatrix.array[offset] = this.cellSize;
      instanceMatrix.array[offset + 5] = this.cellSize;
      instanceMatrix.array[offset + 10] = 1;
      instanceMatrix.array[offset + 12] = x;
      instanceMatrix.array[offset + 13] = y;
      instanceMatrix.array[offset + 15] = 1;

      const isHighlighted = !this.highlight
        || (cell.sourceIndex === this.highlight.sourceIndex && cell.targetIndex === this.highlight.targetIndex);
      const shade = cellShade(cell);
      colorAttr.array.set(isHighlighted ? shade.color.map((c) => Math.min(1, c * 1.35)) : shade.color, index * 3);
      opacityAttr.array[index] = isHighlighted ? 1 : shade.opacity;
    }

    instanceMatrix.needsUpdate = true;
    colorAttr.needsUpdate = true;
    opacityAttr.needsUpdate = true;
    this.geometry.instanceCount = count;
    this.cellCount = count;
    this.renderer.render(this.scene, this.camera);
  }

  dispose() {
    this.geometry.dispose();
    this.material.dispose();
    this.renderer.dispose();
  }
}
