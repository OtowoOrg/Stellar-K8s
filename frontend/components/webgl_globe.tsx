/**
 * WebGLGlobe – interactive 3-D globe component built on plain Three.js.
 *
 * Renders a sphere representing the Earth together with:
 *  - A graticule wireframe grid for geographic context
 *  - Instanced peer-node dots placed at geographic coordinates
 *  - A host node marker (golden)
 *  - Glowing quadratic-Bézier arcs from the host to each peer, coloured by
 *    latency (green < 50 ms → yellow ≤ 200 ms → red > 200 ms)
 *
 * Performance target: ≥ 60 FPS with 50+ simultaneous arcs on standard hardware.
 * The arc geometry uses a single pre-allocated Float32Array buffer that is
 * re-filled without GC pressure on each data update.
 *
 * @module components/webgl_globe
 */

import { useEffect, useRef, useCallback } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';

// ---------------------------------------------------------------------------
// Types (JSDoc-only – the file is .tsx so TypeScript will check consumers)
// ---------------------------------------------------------------------------

export interface GeoCoord {
  lat: number;
  lng: number;
}

export interface ArcData {
  /** Unique identifier for the peer. */
  peerId: string;
  /** Host node coordinates (origin of the arc). */
  from: GeoCoord;
  /** Peer coordinates (destination of the arc). */
  to: GeoCoord;
  /** Round-trip latency in milliseconds – drives arc colour. */
  latencyMs: number;
  /** Pre-computed 0xRRGGBB colour from `latencyColor()`. */
  color: number;
}

export interface WebGLGlobeProps {
  /** Arcs to draw from the host node to its quorum peers. */
  arcs: ArcData[];
  /** Coordinates of the host (this) validator. */
  hostCoord: GeoCoord;
  /** Width in CSS pixels (defaults to container width). */
  width?: number;
  /** Height in CSS pixels (defaults to container height). */
  height?: number;
  /** Accessibility label for screen readers. */
  ariaLabel?: string;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GLOBE_RADIUS     = 1.0;
const GLOBE_SEGMENTS   = 64;
const GLOBE_COLOR      = 0x0d1f2d;
const GRATICULE_COLOR  = 0x1a3a5c;
const HOST_DOT_COLOR   = 0xffd700; // gold
const PEER_DOT_COLOR   = 0xaaaaaa; // grey
const DOT_RADIUS       = 0.018;
const HOST_DOT_RADIUS  = 0.030;
const ARC_SEGMENTS     = 64;       // points per arc curve
const ARC_OPACITY      = 0.85;
const MAX_ARCS         = 256;      // pre-allocated capacity

// ---------------------------------------------------------------------------
// Pure geometry helpers
// ---------------------------------------------------------------------------

/** Converts lat/lng (degrees) to a Three.js Vector3 on the globe surface. */
function latLngToVec3(lat: number, lng: number, radius = GLOBE_RADIUS): THREE.Vector3 {
  const phi   = (90 - lat)  * (Math.PI / 180);
  const theta = (lng + 180) * (Math.PI / 180);
  return new THREE.Vector3(
     Math.sin(phi) * Math.cos(theta) * radius,
     Math.cos(phi) * radius,
    -Math.sin(phi) * Math.sin(theta) * radius,
  );
}

/**
 * Fills a pre-allocated Float32Array with a quadratic Bézier arc between two
 * surface points.  The control point is lifted above the globe surface to
 * produce a visible curve.
 *
 * @returns The number of *vertices* written (ARC_SEGMENTS + 1).
 */
function fillArcBuffer(
  buf: Float32Array,
  offset: number,
  p1: THREE.Vector3,
  p2: THREE.Vector3,
): number {
  const mx = (p1.x + p2.x) * 0.5;
  const my = (p1.y + p2.y) * 0.5;
  const mz = (p1.z + p2.z) * 0.5;
  const midLen = Math.sqrt(mx * mx + my * my + mz * mz) || 1;
  const chord  = p1.distanceTo(p2);
  // Lift the control point off the surface (up to 40 % above radius).
  const lift   = GLOBE_RADIUS + chord * 0.4;
  const cx = (mx / midLen) * lift;
  const cy = (my / midLen) * lift;
  const cz = (mz / midLen) * lift;

  for (let i = 0; i <= ARC_SEGMENTS; i++) {
    const t = i / ARC_SEGMENTS;
    const u = 1 - t;
    const idx = (offset + i) * 3;
    buf[idx]     = u * u * p1.x + 2 * u * t * cx + t * t * p2.x;
    buf[idx + 1] = u * u * p1.y + 2 * u * t * cy + t * t * p2.y;
    buf[idx + 2] = u * u * p1.z + 2 * u * t * cz + t * t * p2.z;
  }
  return ARC_SEGMENTS + 1;
}

// ---------------------------------------------------------------------------
// Graticule builder (latitude / longitude grid lines)
// ---------------------------------------------------------------------------

function buildGraticule(): THREE.LineSegments {
  const positions: number[] = [];
  const r = GLOBE_RADIUS + 0.002; // slightly above surface

  // Latitude lines (every 30°)
  for (let lat = -60; lat <= 60; lat += 30) {
    for (let lng = -180; lng < 180; lng += 3) {
      const a = latLngToVec3(lat, lng, r);
      const b = latLngToVec3(lat, lng + 3, r);
      positions.push(a.x, a.y, a.z, b.x, b.y, b.z);
    }
  }
  // Longitude lines (every 30°)
  for (let lng = -180; lng < 180; lng += 30) {
    for (let lat = -90; lat < 90; lat += 3) {
      const a = latLngToVec3(lat, lng, r);
      const b = latLngToVec3(lat + 3, lng, r);
      positions.push(a.x, a.y, a.z, b.x, b.y, b.z);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
  const mat = new THREE.LineBasicMaterial({
    color: GRATICULE_COLOR,
    transparent: true,
    opacity: 0.4,
  });
  return new THREE.LineSegments(geo, mat);
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function WebGLGlobe({
  arcs,
  hostCoord,
  width,
  height,
  ariaLabel = 'Interactive 3D geospatial quorum and latency map',
}: WebGLGlobeProps) {
  const mountRef = useRef<HTMLDivElement>(null);

  // We store all mutable Three.js state in a ref to avoid re-creating the
  // renderer on every prop update.
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: InstanceType<typeof OrbitControls>;
    arcBuf: Float32Array;
    arcGeo: THREE.BufferGeometry;
    arcMat: THREE.LineBasicMaterial;
    arcLines: THREE.LineSegments;
    arcColorAttr: THREE.Float32BufferAttribute;
    arcPosAttr: THREE.Float32BufferAttribute;
    hostMesh: THREE.Mesh;
    peerMesh: THREE.InstancedMesh;
    frameId: number;
    observer: ResizeObserver;
  } | null>(null);

  // Keep a stable ref to the latest arc data so the animation loop can read
  // it without needing to be re-created on every prop change.
  const arcsRef = useRef<ArcData[]>(arcs);
  const hostRef = useRef<GeoCoord>(hostCoord);
  useEffect(() => { arcsRef.current = arcs; }, [arcs]);
  useEffect(() => { hostRef.current = hostCoord; }, [hostCoord]);

  // -------------------------------------------------------------------------
  // Scene update (called every frame)
  // -------------------------------------------------------------------------
  const updateArcs = useCallback(() => {
    const state = stateRef.current;
    if (!state) return;

    const currentArcs = arcsRef.current;
    const host        = hostRef.current;
    const arcCount    = Math.min(currentArcs.length, MAX_ARCS);
    const hostVec     = latLngToVec3(host.lat, host.lng);

    const { arcBuf, arcColorAttr, arcPosAttr, arcLines, peerMesh } = state;

    // Update arc vertex buffer and per-vertex colours.
    let vertexOffset = 0;
    for (let i = 0; i < arcCount; i++) {
      const arc     = currentArcs[i];
      const peerVec = latLngToVec3(arc.to.lat, arc.to.lng);
      const written = fillArcBuffer(arcBuf, vertexOffset, hostVec, peerVec);

      // Derive RGB components from the hex colour.
      const r = ((arc.color >> 16) & 0xff) / 255;
      const g = ((arc.color >>  8) & 0xff) / 255;
      const b = ( arc.color        & 0xff) / 255;

      for (let v = 0; v < written; v++) {
        const ci = (vertexOffset + v) * 3;
        arcColorAttr.array[ci]     = r;
        arcColorAttr.array[ci + 1] = g;
        arcColorAttr.array[ci + 2] = b;
      }
      vertexOffset += written;
    }

    // Sync positions into the geometry.
    for (let i = 0; i < vertexOffset * 3; i++) {
      arcPosAttr.array[i] = arcBuf[i];
    }
    arcPosAttr.needsUpdate    = true;
    arcColorAttr.needsUpdate  = true;
    arcLines.geometry.setDrawRange(0, vertexOffset);

    // Update peer dot positions.
    const dummy = new THREE.Object3D();
    const peerCount = Math.min(currentArcs.length, MAX_ARCS);
    peerMesh.count = peerCount;
    for (let i = 0; i < peerCount; i++) {
      const v = latLngToVec3(currentArcs[i].to.lat, currentArcs[i].to.lng, GLOBE_RADIUS + 0.005);
      dummy.position.set(v.x, v.y, v.z);
      dummy.updateMatrix();
      peerMesh.setMatrixAt(i, dummy.matrix);
    }
    peerMesh.instanceMatrix.needsUpdate = true;

    // Move host dot.
    const hv = latLngToVec3(host.lat, host.lng, GLOBE_RADIUS + 0.006);
    state.hostMesh.position.set(hv.x, hv.y, hv.z);
  }, []);

  // -------------------------------------------------------------------------
  // Scene initialisation  (runs once on mount)
  // -------------------------------------------------------------------------
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    // Scene + camera
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x0b1119);

    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.position.set(0, 0, 3);

    // Renderer
    const renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance' });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    mount.appendChild(renderer.domElement);

    // Orbit controls
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping   = true;
    controls.dampingFactor   = 0.08;
    controls.minDistance     = 1.2;
    controls.maxDistance     = 8;
    controls.autoRotate      = true;
    controls.autoRotateSpeed = 0.3;
    controls.target.set(0, 0, 0);

    // Lighting
    const ambient = new THREE.AmbientLight(0xffffff, 1.2);
    const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
    dirLight.position.set(5, 3, 5);
    scene.add(ambient, dirLight);

    // Globe sphere
    const globeGeo = new THREE.SphereGeometry(GLOBE_RADIUS, GLOBE_SEGMENTS, GLOBE_SEGMENTS);
    const globeMat = new THREE.MeshPhongMaterial({
      color: GLOBE_COLOR,
      shininess: 30,
      transparent: true,
      opacity: 0.95,
    });
    const globe = new THREE.Mesh(globeGeo, globeMat);
    scene.add(globe);

    // Graticule
    scene.add(buildGraticule());

    // Arc geometry (pre-allocated for MAX_ARCS arcs × (ARC_SEGMENTS+1) vertices)
    const maxVerts   = MAX_ARCS * (ARC_SEGMENTS + 1);
    const arcBuf     = new Float32Array(maxVerts * 3);
    const arcColors  = new Float32Array(maxVerts * 3);

    const arcPosAttr   = new THREE.Float32BufferAttribute(new Float32Array(maxVerts * 3), 3);
    const arcColorAttr = new THREE.Float32BufferAttribute(arcColors, 3);
    arcPosAttr.setUsage(THREE.DynamicDrawUsage);
    arcColorAttr.setUsage(THREE.DynamicDrawUsage);

    const arcGeo = new THREE.BufferGeometry();
    arcGeo.setAttribute('position', arcPosAttr);
    arcGeo.setAttribute('color', arcColorAttr);
    arcGeo.setDrawRange(0, 0);

    const arcMat = new THREE.LineBasicMaterial({
      vertexColors: true,
      transparent: true,
      opacity: ARC_OPACITY,
      linewidth: 1, // > 1 not supported on WebGL without LineMaterial
    });

    const arcLines = new THREE.LineSegments(arcGeo, arcMat);
    arcLines.frustumCulled = false;
    scene.add(arcLines);

    // Host node marker
    const hostGeo = new THREE.SphereGeometry(HOST_DOT_RADIUS, 12, 8);
    const hostMat = new THREE.MeshBasicMaterial({ color: HOST_DOT_COLOR });
    const hostMesh = new THREE.Mesh(hostGeo, hostMat);
    scene.add(hostMesh);

    // Peer node markers (instanced for performance)
    const peerGeo  = new THREE.SphereGeometry(DOT_RADIUS, 8, 6);
    const peerMat  = new THREE.MeshBasicMaterial({ color: PEER_DOT_COLOR });
    const peerMesh = new THREE.InstancedMesh(peerGeo, peerMat, MAX_ARCS);
    peerMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    peerMesh.frustumCulled = false;
    peerMesh.count = 0;
    scene.add(peerMesh);

    // Resize handling
    const resize = () => {
      const w = mount.clientWidth  || 800;
      const h = mount.clientHeight || 600;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h, false);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(mount);
    resize();

    // Store state
    stateRef.current = {
      renderer, scene, camera, controls,
      arcBuf, arcGeo, arcMat, arcLines,
      arcColorAttr, arcPosAttr,
      hostMesh, peerMesh,
      frameId: 0,
      observer,
    };

    // Animation loop
    const animate = () => {
      stateRef.current!.frameId = requestAnimationFrame(animate);
      updateArcs();
      controls.update();
      renderer.render(scene, camera);
    };
    stateRef.current.frameId = requestAnimationFrame(animate);

    // Cleanup
    return () => {
      const s = stateRef.current;
      if (!s) return;
      cancelAnimationFrame(s.frameId);
      s.observer.disconnect();
      s.controls.dispose();
      s.arcGeo.dispose();
      s.arcMat.dispose();
      globeGeo.dispose();
      globeMat.dispose();
      s.peerMesh.geometry.dispose();
      (s.peerMesh.material as THREE.Material).dispose();
      hostGeo.dispose();
      hostMat.dispose();
      s.renderer.dispose();
      mount.removeChild(s.renderer.domElement);
      stateRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // intentionally empty – the loop reads from refs

  // -------------------------------------------------------------------------
  // Explicit size override support
  // -------------------------------------------------------------------------
  const style: React.CSSProperties = {
    width:  width  ? `${width}px`  : '100%',
    height: height ? `${height}px` : '100%',
    display: 'block',
    overflow: 'hidden',
  };

  return (
    <div
      ref={mountRef}
      style={style}
      role="img"
      aria-label={ariaLabel}
      data-testid="webgl-globe"
    />
  );
}
