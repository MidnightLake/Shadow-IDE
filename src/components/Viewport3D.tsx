import React, { useRef, useEffect, useCallback, useState } from "react";

/* ─── Types ─── */

interface Vec3 {
  x: number;
  y: number;
  z: number;
}

interface SceneEntity {
  id: string;
  name: string;
  position: Vec3;
  scale: Vec3;
  color: [number, number, number];
  kind: "cube" | "sphere" | "terrain" | "light" | "camera";
}

interface Props {
  entities: SceneEntity[];
  selectedEntityId?: string | null;
  onSelectEntity?: (id: string | null) => void;
  style?: React.CSSProperties;
}

/* ─── Math helpers ─── */

function mat4Identity(): Float32Array {
  const m = new Float32Array(16);
  m[0] = m[5] = m[10] = m[15] = 1;
  return m;
}

function mat4Perspective(fov: number, aspect: number, near: number, far: number): Float32Array {
  const m = new Float32Array(16);
  const f = 1.0 / Math.tan(fov / 2);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) / (near - far);
  m[11] = -1;
  m[14] = (2 * far * near) / (near - far);
  return m;
}

function mat4LookAt(eye: Vec3, target: Vec3, up: Vec3): Float32Array {
  const zx = eye.x - target.x, zy = eye.y - target.y, zz = eye.z - target.z;
  let len = Math.hypot(zx, zy, zz);
  const fz = { x: zx / len, y: zy / len, z: zz / len };
  const sx = up.y * fz.z - up.z * fz.y;
  const sy = up.z * fz.x - up.x * fz.z;
  const sz = up.x * fz.y - up.y * fz.x;
  len = Math.hypot(sx, sy, sz);
  const fx = { x: sx / len, y: sy / len, z: sz / len };
  const ux = fz.y * fx.z - fz.z * fx.y;
  const uy = fz.z * fx.x - fz.x * fx.z;
  const uz = fz.x * fx.y - fz.y * fx.x;
  const m = new Float32Array(16);
  m[0] = fx.x; m[1] = ux;   m[2] = fz.x; m[3] = 0;
  m[4] = fx.y; m[5] = uy;   m[6] = fz.y; m[7] = 0;
  m[8] = fx.z; m[9] = uz;   m[10] = fz.z; m[11] = 0;
  m[12] = -(fx.x * eye.x + fx.y * eye.y + fx.z * eye.z);
  m[13] = -(ux * eye.x + uy * eye.y + uz * eye.z);
  m[14] = -(fz.x * eye.x + fz.y * eye.y + fz.z * eye.z);
  m[15] = 1;
  return m;
}

function mat4Multiply(a: Float32Array, b: Float32Array): Float32Array {
  const o = new Float32Array(16);
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 4; j++) {
      o[j * 4 + i] = a[i] * b[j * 4] + a[4 + i] * b[j * 4 + 1] + a[8 + i] * b[j * 4 + 2] + a[12 + i] * b[j * 4 + 3];
    }
  }
  return o;
}

function mat4Translate(x: number, y: number, z: number): Float32Array {
  const m = mat4Identity();
  m[12] = x; m[13] = y; m[14] = z;
  return m;
}

function mat4Scale(x: number, y: number, z: number): Float32Array {
  const m = mat4Identity();
  m[0] = x; m[5] = y; m[10] = z;
  return m;
}

/* ─── Geometry generators ─── */

function createCubeGeometry(): { verts: Float32Array; normals: Float32Array; indices: Uint16Array } {
  const p = 0.5;
  const faces = [
    // front
    [-p, -p, p], [p, -p, p], [p, p, p], [-p, p, p],
    // back
    [p, -p, -p], [-p, -p, -p], [-p, p, -p], [p, p, -p],
    // top
    [-p, p, p], [p, p, p], [p, p, -p], [-p, p, -p],
    // bottom
    [-p, -p, -p], [p, -p, -p], [p, -p, p], [-p, -p, p],
    // right
    [p, -p, p], [p, -p, -p], [p, p, -p], [p, p, p],
    // left
    [-p, -p, -p], [-p, -p, p], [-p, p, p], [-p, p, -p],
  ];
  const faceNormals = [
    [0, 0, 1], [0, 0, -1], [0, 1, 0], [0, -1, 0], [1, 0, 0], [-1, 0, 0],
  ];
  const verts: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];
  for (let f = 0; f < 6; f++) {
    const n = faceNormals[f];
    for (let v = 0; v < 4; v++) {
      const pt = faces[f * 4 + v];
      verts.push(pt[0], pt[1], pt[2]);
      normals.push(n[0], n[1], n[2]);
    }
    const base = f * 4;
    indices.push(base, base + 1, base + 2, base, base + 2, base + 3);
  }
  return { verts: new Float32Array(verts), normals: new Float32Array(normals), indices: new Uint16Array(indices) };
}

function createSphereGeometry(segments = 16, rings = 12): { verts: Float32Array; normals: Float32Array; indices: Uint16Array } {
  const verts: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];
  for (let r = 0; r <= rings; r++) {
    const phi = (r / rings) * Math.PI;
    for (let s = 0; s <= segments; s++) {
      const theta = (s / segments) * Math.PI * 2;
      const x = Math.sin(phi) * Math.cos(theta);
      const y = Math.cos(phi);
      const z = Math.sin(phi) * Math.sin(theta);
      verts.push(x * 0.5, y * 0.5, z * 0.5);
      normals.push(x, y, z);
    }
  }
  for (let r = 0; r < rings; r++) {
    for (let s = 0; s < segments; s++) {
      const a = r * (segments + 1) + s;
      const b = a + segments + 1;
      indices.push(a, b, a + 1, a + 1, b, b + 1);
    }
  }
  return { verts: new Float32Array(verts), normals: new Float32Array(normals), indices: new Uint16Array(indices) };
}

function createGridGeometry(size: number, divisions: number): Float32Array {
  const lines: number[] = [];
  const half = size / 2;
  const step = size / divisions;
  for (let i = 0; i <= divisions; i++) {
    const pos = -half + i * step;
    lines.push(pos, 0, -half, pos, 0, half);
    lines.push(-half, 0, pos, half, 0, pos);
  }
  return new Float32Array(lines);
}

function createTerrainGeometry(cols: number, rows: number, scale: number, heightScale: number, frequency: number): { verts: Float32Array; normals: Float32Array; indices: Uint16Array; colors: Float32Array } {
  const verts: number[] = [];
  const normals: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  const halfX = (cols * scale) / 2;
  const halfZ = (rows * scale) / 2;

  for (let r = 0; r <= rows; r++) {
    for (let c = 0; c <= cols; c++) {
      const x = c * scale - halfX;
      const z = r * scale - halfZ;
      const h = Math.sin(x * frequency * 0.1) * Math.cos(z * frequency * 0.1) * heightScale;
      verts.push(x, h, z);
      normals.push(0, 1, 0);
      const t = (h / heightScale + 1) * 0.5;
      colors.push(0.15 + t * 0.3, 0.4 + t * 0.4, 0.15 + t * 0.1);
    }
  }

  // Recompute normals from cross products
  const w = cols + 1;
  for (let r = 0; r <= rows; r++) {
    for (let c = 0; c <= cols; c++) {
      const idx = r * w + c;
      const cx = c < cols ? verts[(idx + 1) * 3] - verts[idx * 3] : verts[idx * 3] - verts[(idx - 1) * 3];
      const cy = c < cols ? verts[(idx + 1) * 3 + 1] - verts[idx * 3 + 1] : verts[idx * 3 + 1] - verts[(idx - 1) * 3 + 1];
      const rIdx = r < rows ? idx + w : idx;
      const rPrev = r < rows ? idx : idx - w;
      const rz = verts[rIdx * 3 + 2] - verts[rPrev * 3 + 2];
      const ry = verts[rIdx * 3 + 1] - verts[rPrev * 3 + 1];
      const nx = -cy * rz;
      const ny = cx * rz - 0 + 1;
      const nz = -cx * ry;
      const len = Math.hypot(nx, ny, nz) || 1;
      normals[idx * 3] = nx / len;
      normals[idx * 3 + 1] = ny / len;
      normals[idx * 3 + 2] = nz / len;
    }
  }

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const a = r * w + c;
      const b = a + 1;
      const d = a + w;
      const e = d + 1;
      indices.push(a, d, b, b, d, e);
    }
  }
  return { verts: new Float32Array(verts), normals: new Float32Array(normals), indices: new Uint16Array(indices), colors: new Float32Array(colors) };
}

/* ─── Shaders ─── */

const MESH_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aNorm;
layout(location=2) in vec3 aColor;
uniform mat4 uProj;
uniform mat4 uView;
uniform mat4 uModel;
uniform int uUseVertColor;
out vec3 vNorm;
out vec3 vWorldPos;
out vec3 vColor;
void main(){
  vec4 world = uModel * vec4(aPos, 1.0);
  vWorldPos = world.xyz;
  vNorm = mat3(uModel) * aNorm;
  vColor = uUseVertColor == 1 ? aColor : vec3(1.0);
  gl_Position = uProj * uView * world;
}`;

const MESH_FRAG = `#version 300 es
precision highp float;
in vec3 vNorm;
in vec3 vWorldPos;
in vec3 vColor;
uniform vec3 uObjColor;
uniform vec3 uLightDir;
uniform float uAmbient;
uniform int uSelected;
uniform int uUseVertColor;
out vec4 fragColor;
void main(){
  vec3 n = normalize(vNorm);
  float diff = max(dot(n, normalize(uLightDir)), 0.0);
  vec3 base = uUseVertColor == 1 ? vColor : uObjColor;
  vec3 lit = base * (uAmbient + diff * (1.0 - uAmbient));
  if(uSelected == 1){
    lit = mix(lit, vec3(1.0, 0.67, 0.2), 0.25);
  }
  fragColor = vec4(lit, 1.0);
}`;

const GRID_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPos;
uniform mat4 uProj;
uniform mat4 uView;
out float vDist;
void main(){
  gl_Position = uProj * uView * vec4(aPos, 1.0);
  vDist = length(aPos.xz);
}`;

const GRID_FRAG = `#version 300 es
precision highp float;
in float vDist;
uniform vec3 uColor;
out vec4 fragColor;
void main(){
  float alpha = smoothstep(50.0, 10.0, vDist) * 0.35;
  fragColor = vec4(uColor, alpha);
}`;

const AXIS_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aCol;
uniform mat4 uProj;
uniform mat4 uView;
out vec3 vCol;
void main(){
  gl_Position = uProj * uView * vec4(aPos, 1.0);
  vCol = aCol;
}`;

const AXIS_FRAG = `#version 300 es
precision highp float;
in vec3 vCol;
out vec4 fragColor;
void main(){
  fragColor = vec4(vCol, 1.0);
}`;

/* ─── WebGL helpers ─── */

function compileShader(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.error("Shader error:", gl.getShaderInfoLog(s));
  }
  return s;
}

function createProgram(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  gl.attachShader(p, compileShader(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compileShader(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    console.error("Program link error:", gl.getProgramInfoLog(p));
  }
  return p;
}

interface MeshGPU {
  vao: WebGLVertexArrayObject;
  indexCount: number;
}

function uploadMesh(gl: WebGL2RenderingContext, _prog: WebGLProgram, verts: Float32Array, normals: Float32Array, indices: Uint16Array, colors?: Float32Array): MeshGPU {
  const vao = gl.createVertexArray()!;
  gl.bindVertexArray(vao);

  const vb = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, vb);
  gl.bufferData(gl.ARRAY_BUFFER, verts, gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);

  const nb = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, nb);
  gl.bufferData(gl.ARRAY_BUFFER, normals, gl.STATIC_DRAW);
  gl.enableVertexAttribArray(1);
  gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);

  if (colors) {
    const cb = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, cb);
    gl.bufferData(gl.ARRAY_BUFFER, colors, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 3, gl.FLOAT, false, 0, 0);
  }

  const ib = gl.createBuffer()!;
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ib);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);

  gl.bindVertexArray(null);
  return { vao, indexCount: indices.length };
}

/* ─── Component ─── */

export type { SceneEntity };

export default function Viewport3D({ entities, selectedEntityId, onSelectEntity: _onSelectEntity, style }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stateRef = useRef<{
    gl: WebGL2RenderingContext;
    meshProg: WebGLProgram;
    gridProg: WebGLProgram;
    axisProg: WebGLProgram;
    cubeMesh: MeshGPU;
    sphereMesh: MeshGPU;
    terrainMesh: MeshGPU | null;
    gridVao: WebGLVertexArrayObject;
    gridVertCount: number;
    axisVao: WebGLVertexArrayObject;
    camDist: number;
    camTheta: number;
    camPhi: number;
    camTarget: Vec3;
    animFrame: number;
  } | null>(null);
  const entitiesRef = useRef(entities);
  const selectedRef = useRef(selectedEntityId);
  const [fps, setFps] = useState(0);

  entitiesRef.current = entities;
  selectedRef.current = selectedEntityId;

  const initGL = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
    if (!gl) {
      console.error("WebGL2 not available");
      return;
    }

    gl.enable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.clearColor(0.043, 0.07, 0.09, 1.0);

    // Mesh program
    const meshProg = createProgram(gl, MESH_VERT, MESH_FRAG);
    const gridProg = createProgram(gl, GRID_VERT, GRID_FRAG);
    const axisProg = createProgram(gl, AXIS_VERT, AXIS_FRAG);

    // Cube & sphere
    const cube = createCubeGeometry();
    const cubeMesh = uploadMesh(gl, meshProg, cube.verts, cube.normals, cube.indices);
    const sphere = createSphereGeometry();
    const sphereMesh = uploadMesh(gl, meshProg, sphere.verts, sphere.normals, sphere.indices);

    // Grid
    const gridData = createGridGeometry(50, 50);
    const gridVao = gl.createVertexArray()!;
    gl.bindVertexArray(gridVao);
    const gb = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, gb);
    gl.bufferData(gl.ARRAY_BUFFER, gridData, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);

    // Axis lines
    const axisData = new Float32Array([
      0, 0, 0, 1, 0.2, 0.2, 2, 0, 0, 1, 0.2, 0.2,  // X red
      0, 0, 0, 0.2, 1, 0.2, 0, 2, 0, 0.2, 1, 0.2,  // Y green
      0, 0, 0, 0.3, 0.5, 1, 0, 0, 2, 0.3, 0.5, 1,  // Z blue
    ]);
    const axisVao = gl.createVertexArray()!;
    gl.bindVertexArray(axisVao);
    const ab = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, ab);
    gl.bufferData(gl.ARRAY_BUFFER, axisData, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 24, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 24, 12);
    gl.bindVertexArray(null);

    stateRef.current = {
      gl,
      meshProg,
      gridProg,
      axisProg,
      cubeMesh,
      sphereMesh,
      terrainMesh: null,
      gridVao,
      gridVertCount: gridData.length / 3,
      axisVao,
      camDist: 15,
      camTheta: Math.PI / 4,
      camPhi: Math.PI / 5,
      camTarget: { x: 0, y: 0, z: 0 },
      animFrame: 0,
    };

    // Build initial terrain
    rebuildTerrain();

    startRenderLoop();
  }, []);

  const rebuildTerrain = useCallback(() => {
    const s = stateRef.current;
    if (!s) return;
    const terrainEntity = entitiesRef.current.find((e) => e.kind === "terrain");
    if (!terrainEntity) {
      s.terrainMesh = null;
      return;
    }
    const terrain = createTerrainGeometry(32, 32, 0.6, 3.0, 1.0);
    s.terrainMesh = uploadMesh(s.gl, s.meshProg, terrain.verts, terrain.normals, terrain.indices, terrain.colors);
  }, []);

  const startRenderLoop = useCallback(() => {
    let lastTime = performance.now();
    let frameCount = 0;
    let fpsAccum = 0;

    const render = (time: number) => {
      const s = stateRef.current;
      if (!s) return;

      // FPS counter
      frameCount++;
      fpsAccum += time - lastTime;
      lastTime = time;
      if (fpsAccum >= 1000) {
        setFps(frameCount);
        frameCount = 0;
        fpsAccum = 0;
      }

      const canvas = canvasRef.current!;
      const dpr = window.devicePixelRatio || 1;
      const w = canvas.clientWidth * dpr;
      const h = canvas.clientHeight * dpr;
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      const { gl } = s;
      gl.viewport(0, 0, w, h);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

      const aspect = w / h || 1;
      const proj = mat4Perspective(Math.PI / 4, aspect, 0.1, 500);
      const eyeX = s.camTarget.x + s.camDist * Math.sin(s.camPhi) * Math.cos(s.camTheta);
      const eyeY = s.camTarget.y + s.camDist * Math.cos(s.camPhi);
      const eyeZ = s.camTarget.z + s.camDist * Math.sin(s.camPhi) * Math.sin(s.camTheta);
      const view = mat4LookAt({ x: eyeX, y: eyeY, z: eyeZ }, s.camTarget, { x: 0, y: 1, z: 0 });

      // Draw grid
      gl.useProgram(s.gridProg);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.gridProg, "uProj"), false, proj);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.gridProg, "uView"), false, view);
      gl.uniform3f(gl.getUniformLocation(s.gridProg, "uColor"), 0.25, 0.35, 0.4);
      gl.bindVertexArray(s.gridVao);
      gl.drawArrays(gl.LINES, 0, s.gridVertCount);

      // Draw axis
      gl.useProgram(s.axisProg);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.axisProg, "uProj"), false, proj);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.axisProg, "uView"), false, view);
      gl.bindVertexArray(s.axisVao);
      gl.lineWidth(2);
      gl.drawArrays(gl.LINES, 0, 6);

      // Draw terrain
      if (s.terrainMesh) {
        gl.useProgram(s.meshProg);
        gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uProj"), false, proj);
        gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uView"), false, view);
        gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uModel"), false, mat4Identity());
        gl.uniform3f(gl.getUniformLocation(s.meshProg, "uObjColor"), 0.3, 0.6, 0.3);
        gl.uniform3f(gl.getUniformLocation(s.meshProg, "uLightDir"), 0.4, 0.8, 0.3);
        gl.uniform1f(gl.getUniformLocation(s.meshProg, "uAmbient"), 0.3);
        gl.uniform1i(gl.getUniformLocation(s.meshProg, "uSelected"), 0);
        gl.uniform1i(gl.getUniformLocation(s.meshProg, "uUseVertColor"), 1);
        gl.bindVertexArray(s.terrainMesh.vao);
        gl.drawElements(gl.TRIANGLES, s.terrainMesh.indexCount, gl.UNSIGNED_SHORT, 0);
      }

      // Draw entities
      gl.useProgram(s.meshProg);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uProj"), false, proj);
      gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uView"), false, view);
      gl.uniform3f(gl.getUniformLocation(s.meshProg, "uLightDir"), 0.4, 0.8, 0.3);
      gl.uniform1f(gl.getUniformLocation(s.meshProg, "uAmbient"), 0.3);
      gl.uniform1i(gl.getUniformLocation(s.meshProg, "uUseVertColor"), 0);

      const ents = entitiesRef.current;
      for (const ent of ents) {
        if (ent.kind === "terrain") continue;
        const mesh = ent.kind === "sphere" ? s.sphereMesh : s.cubeMesh;
        const model = mat4Multiply(
          mat4Translate(ent.position.x, ent.position.y, ent.position.z),
          mat4Scale(ent.scale.x, ent.scale.y, ent.scale.z),
        );
        gl.uniformMatrix4fv(gl.getUniformLocation(s.meshProg, "uModel"), false, model);
        gl.uniform3f(gl.getUniformLocation(s.meshProg, "uObjColor"), ent.color[0], ent.color[1], ent.color[2]);
        gl.uniform1i(gl.getUniformLocation(s.meshProg, "uSelected"), ent.id === selectedRef.current ? 1 : 0);
        gl.bindVertexArray(mesh.vao);
        gl.drawElements(gl.TRIANGLES, mesh.indexCount, gl.UNSIGNED_SHORT, 0);
      }

      gl.bindVertexArray(null);
      s.animFrame = requestAnimationFrame(render);
    };

    const s = stateRef.current;
    if (s) {
      s.animFrame = requestAnimationFrame(render);
    }
  }, []);

  // Init
  useEffect(() => {
    initGL();
    return () => {
      if (stateRef.current) {
        cancelAnimationFrame(stateRef.current.animFrame);
      }
    };
  }, [initGL]);

  // Rebuild terrain when entities change
  useEffect(() => {
    rebuildTerrain();
  }, [entities, rebuildTerrain]);

  // Mouse controls: orbit, pan, zoom
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    let dragging = false;
    let panning = false;
    let lastX = 0;
    let lastY = 0;

    const onMouseDown = (e: MouseEvent) => {
      if (e.button === 0) {
        dragging = true;
      } else if (e.button === 1 || e.button === 2) {
        panning = true;
      }
      lastX = e.clientX;
      lastY = e.clientY;
    };

    const onMouseMove = (e: MouseEvent) => {
      const s = stateRef.current;
      if (!s) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      lastX = e.clientX;
      lastY = e.clientY;

      if (dragging) {
        s.camTheta -= dx * 0.008;
        s.camPhi = Math.max(0.1, Math.min(Math.PI - 0.1, s.camPhi - dy * 0.008));
      }
      if (panning) {
        const panSpeed = s.camDist * 0.003;
        const right = {
          x: Math.cos(s.camTheta),
          z: -Math.sin(s.camTheta),
        };
        s.camTarget.x -= right.x * dx * panSpeed;
        s.camTarget.z -= right.z * dx * panSpeed;
        s.camTarget.y += dy * panSpeed;
      }
    };

    const onMouseUp = () => {
      dragging = false;
      panning = false;
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const s = stateRef.current;
      if (!s) return;
      s.camDist = Math.max(1, Math.min(200, s.camDist * (1 + e.deltaY * 0.001)));
    };

    const onContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };

    canvas.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("contextmenu", onContextMenu);

    return () => {
      canvas.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("contextmenu", onContextMenu);
    };
  }, []);

  return (
    <div style={{ position: "relative", width: "100%", height: "100%", ...style }}>
      <canvas
        ref={canvasRef}
        style={{ width: "100%", height: "100%", display: "block", borderRadius: 8 }}
      />
      {/* Overlay HUD */}
      <div style={hudStyle}>
        <span style={{ color: "#e9aa5f", fontWeight: 700 }}>3D Viewport</span>
        <span>{fps} FPS</span>
        <span>{entities.length} objects</span>
      </div>
      <div style={controlsHintStyle}>
        LMB: Orbit &middot; RMB: Pan &middot; Scroll: Zoom
      </div>
    </div>
  );
}

const hudStyle: React.CSSProperties = {
  position: "absolute",
  top: 8,
  left: 10,
  display: "flex",
  gap: 12,
  fontSize: 11,
  color: "#8eb5c4",
  pointerEvents: "none",
  background: "rgba(11,18,23,0.7)",
  padding: "4px 10px",
  borderRadius: 6,
};

const controlsHintStyle: React.CSSProperties = {
  position: "absolute",
  bottom: 8,
  left: "50%",
  transform: "translateX(-50%)",
  fontSize: 10,
  color: "#6f8792",
  pointerEvents: "none",
  background: "rgba(11,18,23,0.7)",
  padding: "3px 10px",
  borderRadius: 4,
};
