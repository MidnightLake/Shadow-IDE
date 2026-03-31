import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SidebarView } from "../types";
import Viewport3D, { type SceneEntity } from "./Viewport3D";

interface Props {
  projectPath?: string;
  visible?: boolean;
  onOpenFile?: (path: string, name: string) => void;
  onActivatePanel?: (view: SidebarView) => void;
  autoBuildManagedExternally?: boolean;
  layoutMode?: "workspace" | "dock";
  onCloseDock?: () => void;
  onRequestFullView?: () => void;
}

interface ShadowProjectInfo {
  name: string;
  runtime: string;
  entry_scene: string;
  entry_scene_path: string;
  game_library_path: string;
  game_library_exists: boolean;
  compiler: string;
  standard: string;
  build_system: string;
}

interface ShadowBuildResult {
  success: boolean;
  output: string;
  duration_ms: number;
}

interface ShadowReflectRaw {
  component_count: number;
  headers_scanned: number;
  json: string;
}

interface ShadowRuntimeStatus {
  project_path: string;
  library_path: string;
  library_exists: boolean;
  is_live: boolean;
  status_line: string;
  frame_index: number;
  component_count: number;
  entity_count: number;
  entry_scene_path: string;
  last_scene_path: string;
  last_error?: string | null;
}

interface ShadowSourceFile {
  path: string;
  kind: string;
}

interface ShadowSceneComponent {
  component_type: string;
  fields: [string, string][];
}

interface ShadowSceneEntity {
  id: string;
  name: string;
  components: ShadowSceneComponent[];
}

interface ShadowScene {
  scene_name: string;
  version: string;
  runtime: string;
  entities: ShadowSceneEntity[];
}

interface WorkspaceFsChangeEvent {
  kind: string;
  paths: string[];
  dir: string;
}

interface ViewportTerrain {
  entityId: string;
  entityName: string;
  componentType: string;
  cols: number;
  rows: number;
  scale: number;
  heightScale: number;
  frequency: number;
  offset: [number, number, number];
}

interface ViewportMarker {
  id: string;
  name: string;
  position: [number, number, number];
  rotation: [number, number, number, number];
  scale: [number, number, number];
  tone: string;
  selected: boolean;
  kind: "light" | "camera" | "player" | "ground" | "entity";
}

interface ViewportBounds {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
  minZ: number;
  maxZ: number;
}

interface ViewportModel {
  terrain: ViewportTerrain | null;
  markers: ViewportMarker[];
  bounds: ViewportBounds;
  lightDirection: [number, number, number];
}

type ViewportCameraMode = "orbit" | "third_person" | "first_person";

interface Vec2Point {
  x: number;
  y: number;
}

const COMMON_COMPONENT_TYPES = [
  "Transform",
  "MeshRenderer",
  "SpriteRenderer",
  "RigidBody",
  "Collider",
  "PlayerController",
  "Lifetime",
  "Health",
] as const;

function resolveProjectPath(projectPath: string, relativePath: string): string {
  if (relativePath.startsWith("/") || /^[A-Za-z]:[\\/]/.test(relativePath)) {
    return relativePath;
  }
  const base = projectPath.replace(/[\\/]+$/, "");
  return `${base}/${relativePath}`.replace(/\\/g, "/");
}

function pickPrimaryFile(files: ShadowSourceFile[], preferred: string[]): ShadowSourceFile | null {
  for (const candidate of preferred) {
    const found = files.find((file) => file.path === candidate);
    if (found) return found;
  }
  return files[0] ?? null;
}

function sceneFieldDraftKey(entityId: string, componentType: string, fieldName: string): string {
  return `${entityId}::${componentType}::${fieldName}`;
}

function sceneEntityDraftKey(entityId: string): string {
  return `${entityId}::name`;
}

function reflectionComponentNames(raw: ShadowReflectRaw): string[] {
  try {
    const parsed = JSON.parse(raw.json) as {
      components?: Array<{ name?: string }>;
    };
    return (parsed.components ?? [])
      .map((component) => component.name?.trim() ?? "")
      .filter(Boolean);
  } catch {
    return [];
  }
}

function normalizeToken(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "");
}

function normalizeFsPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+/g, "/");
}

function isAutoBuildTriggerPath(relativePath: string): boolean {
  const normalized = relativePath.replace(/^\.\/+/, "");
  if (normalized === ".shadow_project.toml" || normalized === "CMakeLists.txt" || normalized === "build.ninja") {
    return true;
  }
  if (!(normalized.startsWith("src/") || normalized.startsWith("game/"))) {
    return false;
  }
  return /\.(h|hpp|cpp|cxx|cc)$/i.test(normalized);
}

function isSceneRefreshPath(relativePath: string): boolean {
  const normalized = relativePath.replace(/^\.\/+/, "");
  return normalized.startsWith("scenes/") || normalized.startsWith("assets/") || normalized === ".shadow_project.toml";
}

function componentMatches(component: ShadowSceneComponent, token: string): boolean {
  const normalized = normalizeToken(component.component_type);
  return normalized === token || normalized.endsWith(token);
}

function findComponent(entity: ShadowSceneEntity, token: string): ShadowSceneComponent | null {
  return entity.components.find((component) => componentMatches(component, token)) ?? null;
}

function getFieldValue(component: ShadowSceneComponent | null, names: string[]): string | null {
  if (!component) return null;
  const normalizedNames = names.map(normalizeToken);
  for (const [fieldName, fieldValue] of component.fields) {
    if (normalizedNames.includes(normalizeToken(fieldName))) {
      return fieldValue;
    }
  }
  return null;
}

function parseNumberList(value: string | null | undefined): number[] {
  if (!value) return [];
  const matches = value.match(/-?\d*\.?\d+(?:e[-+]?\d+)?/gi) ?? [];
  return matches
    .map((match) => Number.parseFloat(match))
    .filter((number) => Number.isFinite(number));
}

function parseNumberValue(value: string | null | undefined, fallback: number): number {
  const number = parseNumberList(value)[0];
  return Number.isFinite(number) ? number : fallback;
}

function parseVector3(value: string | null | undefined): [number, number, number] | null {
  const numbers = parseNumberList(value);
  if (numbers.length >= 3) {
    return [numbers[0], numbers[1], numbers[2]];
  }
  if (numbers.length === 2) {
    return [numbers[0], 0, numbers[1]];
  }
  return null;
}

function parseQuaternion(value: string | null | undefined): [number, number, number, number] | null {
  const numbers = parseNumberList(value);
  if (numbers.length >= 4) {
    return [numbers[0], numbers[1], numbers[2], numbers[3]];
  }
  return null;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function vec3Length(vector: [number, number, number]): number {
  return Math.hypot(vector[0], vector[1], vector[2]);
}

function vec3Normalize(vector: [number, number, number]): [number, number, number] {
  const length = vec3Length(vector);
  if (length <= 1e-5) {
    return [0, 1, 0];
  }
  return [vector[0] / length, vector[1] / length, vector[2] / length];
}

function vec3Add(left: [number, number, number], right: [number, number, number]): [number, number, number] {
  return [left[0] + right[0], left[1] + right[1], left[2] + right[2]];
}

function vec3Subtract(left: [number, number, number], right: [number, number, number]): [number, number, number] {
  return [left[0] - right[0], left[1] - right[1], left[2] - right[2]];
}

function vec3Scale(vector: [number, number, number], scalar: number): [number, number, number] {
  return [vector[0] * scalar, vector[1] * scalar, vector[2] * scalar];
}

function vec3Dot(left: [number, number, number], right: [number, number, number]): number {
  return left[0] * right[0] + left[1] * right[1] + left[2] * right[2];
}

function vec3Cross(left: [number, number, number], right: [number, number, number]): [number, number, number] {
  return [
    left[1] * right[2] - left[2] * right[1],
    left[2] * right[0] - left[0] * right[2],
    left[0] * right[1] - left[1] * right[0],
  ];
}

function vec3Average(points: Array<[number, number, number]>): [number, number, number] {
  if (points.length === 0) {
    return [0, 0, 0];
  }
  const sum = points.reduce<[number, number, number]>((accumulator, point) => [
    accumulator[0] + point[0],
    accumulator[1] + point[1],
    accumulator[2] + point[2],
  ], [0, 0, 0]);
  return [sum[0] / points.length, sum[1] / points.length, sum[2] / points.length];
}

function normalizeQuaternion(quaternion: [number, number, number, number]): [number, number, number, number] {
  const length = Math.hypot(quaternion[0], quaternion[1], quaternion[2], quaternion[3]);
  if (length <= 1e-5) {
    return [0, 0, 0, 1];
  }
  return [
    quaternion[0] / length,
    quaternion[1] / length,
    quaternion[2] / length,
    quaternion[3] / length,
  ];
}

function rotateVectorByQuaternion(
  vector: [number, number, number],
  quaternion: [number, number, number, number],
): [number, number, number] {
  const [x, y, z, w] = normalizeQuaternion(quaternion);
  const uv = vec3Cross([x, y, z], vector);
  const uuv = vec3Cross([x, y, z], uv);
  return vec3Add(vector, vec3Add(vec3Scale(uv, 2 * w), vec3Scale(uuv, 2)));
}

function directionToYawPitch(direction: [number, number, number]): { yaw: number; pitch: number } {
  const normalized = vec3Normalize(direction);
  return {
    yaw: Math.atan2(normalized[0], normalized[2] || 1e-5),
    pitch: Math.asin(clamp(normalized[1], -0.98, 0.98)),
  };
}

function yawPitchToDirection(yaw: number, pitch: number): [number, number, number] {
  const cosPitch = Math.cos(pitch);
  return vec3Normalize([
    Math.sin(yaw) * cosPitch,
    Math.sin(pitch),
    Math.cos(yaw) * cosPitch,
  ]);
}

function colorToRgb(color: string): [number, number, number] {
  const normalized = color.trim();
  const hex = normalized.startsWith("#") ? normalized.slice(1) : normalized;
  if (hex.length === 3) {
    return [
      Number.parseInt(hex[0] + hex[0], 16),
      Number.parseInt(hex[1] + hex[1], 16),
      Number.parseInt(hex[2] + hex[2], 16),
    ];
  }
  if (hex.length === 6) {
    return [
      Number.parseInt(hex.slice(0, 2), 16),
      Number.parseInt(hex.slice(2, 4), 16),
      Number.parseInt(hex.slice(4, 6), 16),
    ];
  }
  return [216, 226, 231];
}

function shadeColor(color: string, intensity: number, alpha = 1): string {
  const [red, green, blue] = colorToRgb(color);
  const shaded = clamp(intensity, 0, 1.45);
  const ambientLift = 22;
  return `rgba(${Math.round(clamp(red * shaded + ambientLift, 0, 255))}, ${Math.round(clamp(green * shaded + ambientLift, 0, 255))}, ${Math.round(clamp(blue * shaded + ambientLift, 0, 255))}, ${alpha})`;
}

function lerp(start: number, end: number, amount: number): number {
  return start + (end - start) * amount;
}

function smoothstep(min: number, max: number, value: number): number {
  if (min === max) {
    return value < min ? 0 : 1;
  }
  const t = clamp((value - min) / (max - min), 0, 1);
  return t * t * (3 - 2 * t);
}

function mixColor(left: string, right: string, amount: number, alpha = 1): string {
  const [leftRed, leftGreen, leftBlue] = colorToRgb(left);
  const [rightRed, rightGreen, rightBlue] = colorToRgb(right);
  const t = clamp(amount, 0, 1);
  return `rgba(${Math.round(lerp(leftRed, rightRed, t))}, ${Math.round(lerp(leftGreen, rightGreen, t))}, ${Math.round(lerp(leftBlue, rightBlue, t))}, ${alpha})`;
}

function seededUnit(seed: number): number {
  const value = Math.sin(seed * 127.1 + 311.7) * 43758.5453123;
  return value - Math.floor(value);
}

function seededSigned(seed: number): number {
  return seededUnit(seed) * 2 - 1;
}

function clampGrid(value: number, fallback: number): number {
  const rounded = Math.round(value);
  return Number.isFinite(rounded) ? Math.max(8, Math.min(192, rounded)) : fallback;
}

function terrainHeightAt(x: number, z: number, heightScale: number, frequency: number): number {
  const f = frequency * 0.3;
  return heightScale * (
    Math.sin(x * f) * 0.5
    + Math.cos(z * f) * 0.5
    + Math.sin(x * f * 2.1 + 1.3) * Math.cos(z * f * 1.7) * 0.3
  );
}

function toneForEntity(entity: ShadowSceneEntity): string {
  const tokens = entity.components.map((component) => normalizeToken(component.component_type));
  if (tokens.some((token) => token.includes("light"))) return "#f7c66a";
  if (tokens.some((token) => token.includes("camera"))) return "#8ec7ff";
  if (tokens.some((token) => token.includes("terrain"))) return "#7ed4a7";
  if (tokens.some((token) => token.includes("ground") || token.includes("floor"))) return "#6e8b67";
  if (tokens.some((token) => token.includes("player"))) return "#f58bb7";
  if (tokens.some((token) => token.includes("rigidbody") || token.includes("collider"))) return "#7eb8d4";
  return "#d8e2e7";
}

function classifyViewportKind(entity: ShadowSceneEntity, scale: [number, number, number]): ViewportMarker["kind"] {
  const tokens = [
    normalizeToken(entity.name),
    ...entity.components.map((component) => normalizeToken(component.component_type)),
  ];
  if (tokens.some((token) => token.includes("light"))) return "light";
  if (tokens.some((token) => token.includes("camera"))) return "camera";
  if (tokens.some((token) => token.includes("player") || token.includes("character"))) return "player";
  if (tokens.some((token) => token.includes("ground") || token.includes("floor") || token.includes("terrain"))) return "ground";
  if (scale[1] <= 0.8 && (scale[0] >= 6 || scale[2] >= 6)) return "ground";
  return "entity";
}

function viewportKindPriority(kind: ViewportMarker["kind"]): number {
  switch (kind) {
    case "player":
      return 0;
    case "camera":
      return 1;
    case "entity":
      return 2;
    case "ground":
      return 3;
    case "light":
      return 4;
    default:
      return 5;
  }
}

function preferredSceneEntity(scene: ShadowScene): ShadowSceneEntity | null {
  const decorated = scene.entities.map((entity) => {
    const transform = findComponent(entity, "transform");
    const scale = parseVector3(getFieldValue(transform, ["scale", "size", "extent"])) ?? [1, 1, 1];
    return {
      entity,
      priority: viewportKindPriority(classifyViewportKind(entity, scale)),
    };
  });

  decorated.sort((left, right) => left.priority - right.priority);
  return decorated[0]?.entity ?? scene.entities[0] ?? null;
}

function buildViewportModel(scene: ShadowScene | null, selectedEntityId: string | null): ViewportModel {
  const defaultBounds: ViewportBounds = {
    minX: -6,
    maxX: 6,
    minY: -2,
    maxY: 3,
    minZ: -6,
    maxZ: 6,
  };

  if (!scene) {
    return {
      terrain: null,
      markers: [],
      bounds: defaultBounds,
      lightDirection: vec3Normalize([-0.35, -1, -0.2]),
    };
  }

  let terrain: ViewportTerrain | null = null;
  const markers: ViewportMarker[] = [];
  const bounds: ViewportBounds = { ...defaultBounds };
  let lightDirection: [number, number, number] | null = null;

  const expandBounds = (x: number, y: number, z: number) => {
    bounds.minX = Math.min(bounds.minX, x);
    bounds.maxX = Math.max(bounds.maxX, x);
    bounds.minY = Math.min(bounds.minY, y);
    bounds.maxY = Math.max(bounds.maxY, y);
    bounds.minZ = Math.min(bounds.minZ, z);
    bounds.maxZ = Math.max(bounds.maxZ, z);
  };

  for (const entity of scene.entities) {
    const transform = findComponent(entity, "transform");
    const position = parseVector3(
      getFieldValue(transform, ["position", "translation", "location", "origin"]),
    ) ?? [0, 0, 0];
    const rotation = normalizeQuaternion(
      parseQuaternion(getFieldValue(transform, ["rotation", "orientation", "quaternion"])) ?? [0, 0, 0, 1],
    );
    const scale = parseVector3(
      getFieldValue(transform, ["scale", "size", "extent"]),
    ) ?? [1, 1, 1];
    const terrainComponent = findComponent(entity, "terrain");
    const kind = classifyViewportKind(entity, scale);

    if (terrainComponent && !terrain) {
      const resolutionValue = parseNumberList(getFieldValue(terrainComponent, ["resolution", "grid_size", "grid", "size"]));
      const cols = clampGrid(
        parseNumberValue(getFieldValue(terrainComponent, ["cols", "columns"]), resolutionValue[0] ?? 48),
        48,
      );
      const rows = clampGrid(
        parseNumberValue(getFieldValue(terrainComponent, ["rows"]), resolutionValue[1] ?? resolutionValue[0] ?? 48),
        cols,
      );
      const scale = Math.max(2, parseNumberValue(getFieldValue(terrainComponent, ["scale", "width", "extent"]), 18));
      const heightScale = Math.max(0.1, parseNumberValue(getFieldValue(terrainComponent, ["height_scale", "height", "amplitude"]), 2.4));
      const frequency = Math.max(0.05, parseNumberValue(getFieldValue(terrainComponent, ["frequency", "freq", "noise_frequency"]), 1.6));
      terrain = {
        entityId: entity.id,
        entityName: entity.name || entity.id,
        componentType: terrainComponent.component_type,
        cols,
        rows,
        scale,
        heightScale,
        frequency,
        offset: position,
      };

      const halfScale = scale / 2;
      expandBounds(position[0] - halfScale, position[1] - heightScale, position[2] - halfScale);
      expandBounds(position[0] + halfScale, position[1] + heightScale, position[2] + halfScale);
    }

    if (transform) {
      markers.push({
        id: entity.id,
        name: entity.name || entity.id,
        position,
        rotation,
        scale,
        tone: toneForEntity(entity),
        selected: entity.id === selectedEntityId,
        kind,
      });
      const halfExtents: [number, number, number] = [
        Math.max(0.3, Math.abs(scale[0]) * 0.5),
        Math.max(0.3, Math.abs(scale[1]) * 0.5),
        Math.max(0.3, Math.abs(scale[2]) * 0.5),
      ];
      expandBounds(position[0] - halfExtents[0], position[1] - halfExtents[1], position[2] - halfExtents[2]);
      expandBounds(position[0] + halfExtents[0], position[1] + halfExtents[1], position[2] + halfExtents[2]);
    }

    if (!lightDirection && kind === "light") {
      lightDirection = vec3Normalize(rotateVectorByQuaternion([0, 0, -1], rotation));
    }
  }

  return {
    terrain,
    markers,
    bounds,
    lightDirection: lightDirection ?? vec3Normalize([-0.35, -1, -0.2]),
  };
}

export default function ShadowGameWorkspace({
  projectPath,
  visible,
  onOpenFile,
  onActivatePanel,
  autoBuildManagedExternally = false,
  layoutMode = "workspace",
  onCloseDock,
  onRequestFullView,
}: Props) {
  const [projectInfo, setProjectInfo] = useState<ShadowProjectInfo | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<ShadowRuntimeStatus | null>(null);
  const [sourceFiles, setSourceFiles] = useState<ShadowSourceFile[]>([]);
  const [reflectComponentTypes, setReflectComponentTypes] = useState<string[]>([]);
  const [scene, setScene] = useState<ShadowScene | null>(null);
  const [sceneSource, setSceneSource] = useState<"live" | "entry" | "none">("none");
  const [selectedEntityId, setSelectedEntityId] = useState<string | null>(null);
  const [runtimeDelta, setRuntimeDelta] = useState("0.0167");
  const [buildLog, setBuildLog] = useState("");
  const [building, setBuilding] = useState(false);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimePlaying, setRuntimePlaying] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [sceneBusy, setSceneBusy] = useState(false);
  const [sceneStatus, setSceneStatus] = useState<string | null>(null);
  const [newEntityName, setNewEntityName] = useState("");
  const [newComponentType, setNewComponentType] = useState("");
  const [sceneDrafts, setSceneDrafts] = useState<Record<string, string>>({});
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showBuildLog, setShowBuildLog] = useState(false);
  const [autoBuildEnabled, setAutoBuildEnabled] = useState(true);
  const [autoBuildStatus, setAutoBuildStatus] = useState<string | null>(null);
  const [queuedAutoBuildReason, setQueuedAutoBuildReason] = useState<string | null>(null);
  const autoBuildTimerRef = useRef<number | null>(null);
  const dataRefreshTimerRef = useRef<number | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const pointerDragRef = useRef<{ x: number; y: number } | null>(null);
  const keyStateRef = useRef<Set<string>>(new Set());
  const [viewportFocused, setViewportFocused] = useState(false);
  const [cameraMode, setCameraMode] = useState<ViewportCameraMode>("orbit");
  const [viewportRenderer, setViewportRenderer] = useState<"3d" | "scene">("3d");
  const [orbitYaw, setOrbitYaw] = useState(-0.85);
  const [orbitPitch, setOrbitPitch] = useState(0.48);
  const [orbitDistance, setOrbitDistance] = useState(22);
  const [thirdPersonYawOffset, setThirdPersonYawOffset] = useState(0);
  const [thirdPersonPitch, setThirdPersonPitch] = useState(0.18);
  const [thirdPersonDistance, setThirdPersonDistance] = useState(7.5);
  const [firstPersonYaw, setFirstPersonYaw] = useState(0);
  const [firstPersonPitch, setFirstPersonPitch] = useState(0);
  const [firstPersonPosition, setFirstPersonPosition] = useState<[number, number, number] | null>(null);

  const refreshScene = useCallback(async (info: ShadowProjectInfo, status: ShadowRuntimeStatus | null) => {
    if (!projectPath) return;

    if (status?.is_live) {
      try {
        const liveScene = await invoke<ShadowScene>("shadow_runtime_capture_scene", { projectPath });
        setScene(liveScene);
        setSceneSource("live");
        return;
      } catch {
        // Fall back to the authored entry scene.
      }
    }

    if (info.entry_scene_path) {
      try {
        const parsedScene = await invoke<ShadowScene>("shadow_parse_scene", {
          scenePath: resolveProjectPath(projectPath, info.entry_scene_path),
        });
        setScene(parsedScene);
        setSceneSource("entry");
        return;
      } catch {
        // handled below
      }
    }

    setScene(null);
    setSceneSource("none");
  }, [projectPath]);

  const load = useCallback(async () => {
    if (!projectPath) return;
    setLoadError(null);
    try {
      const info = await invoke<ShadowProjectInfo>("shadow_get_project_info", { projectPath });
      setProjectInfo(info);

      try {
        const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_status", { projectPath });
        setRuntimeStatus(status);
        await refreshScene(info, status);
      } catch {
        setRuntimeStatus(null);
        await refreshScene(info, null);
      }

      try {
        const listedSources = await invoke<ShadowSourceFile[]>("shadow_list_source_files", { projectPath });
        setSourceFiles(listedSources);
      } catch {
        setSourceFiles([]);
      }

      try {
        const raw = await invoke<ShadowReflectRaw>("shadow_load_reflection", { projectPath });
        setReflectComponentTypes(reflectionComponentNames(raw));
      } catch {
        setReflectComponentTypes([]);
      }

      try {
        const lastBuild = await invoke<string>("shadow_get_last_build_log", { projectPath });
        setBuildLog(lastBuild);
      } catch {
        setBuildLog("");
      }
    } catch (error) {
      const message = String(error);
      if (message.includes("shadow_project.toml") || message.includes("No such file") || message.includes("os error 2")) {
        setProjectInfo(null);
        setRuntimeStatus(null);
        setScene(null);
        setSceneSource("none");
        setBuildLog("");
        setSourceFiles([]);
        setReflectComponentTypes([]);
        return;
      }
      setLoadError(message);
    }
  }, [projectPath, refreshScene]);

  useEffect(() => {
    setProjectInfo(null);
    setRuntimeStatus(null);
    setScene(null);
    setSceneSource("none");
    setSelectedEntityId(null);
    setBuildLog("");
    setSourceFiles([]);
    setReflectComponentTypes([]);
    setRuntimePlaying(false);
    setLoadError(null);
    setSceneBusy(false);
    setSceneStatus(null);
    setNewEntityName("");
    setNewComponentType("");
    setSceneDrafts({});
    setAutoBuildStatus(null);
    setQueuedAutoBuildReason(null);
    setViewportFocused(false);
    setCameraMode("orbit");
    setOrbitYaw(-0.85);
    setOrbitPitch(0.48);
    setOrbitDistance(22);
    setThirdPersonYawOffset(0);
    setThirdPersonPitch(0.18);
    setThirdPersonDistance(7.5);
    setFirstPersonYaw(0);
    setFirstPersonPitch(0);
    setFirstPersonPosition(null);
  }, [projectPath]);

  useEffect(() => {
    if (visible) {
      void load();
    }
  }, [visible, load]);

  useEffect(() => {
    if (!scene?.entities.length) {
      setSelectedEntityId(null);
      return;
    }
    setSelectedEntityId((current) => {
      if (current && scene.entities.some((entity) => entity.id === current)) {
        return current;
      }
      return preferredSceneEntity(scene)?.id ?? scene.entities[0]?.id ?? null;
    });
  }, [scene]);

  useEffect(() => {
    if (!runtimeStatus?.is_live && runtimePlaying) {
      setRuntimePlaying(false);
    }
  }, [runtimePlaying, runtimeStatus?.is_live]);

  const viewportModel = useMemo(
    () => buildViewportModel(scene, selectedEntityId),
    [scene, selectedEntityId],
  );
  const primaryHeader = useMemo(() => pickPrimaryFile(
    sourceFiles.filter((file) => file.kind === "header"),
    ["src/game.h", "game/game.h"],
  ), [sourceFiles]);
  const primarySource = useMemo(() => pickPrimaryFile(
    sourceFiles.filter((file) => file.kind === "source"),
    ["src/game.cpp", "game/game.cpp"],
  ), [sourceFiles]);
  const selectedEntity = scene?.entities.find((entity) => entity.id === selectedEntityId) ?? scene?.entities[0] ?? null;
  const availableComponentTypes = useMemo(() => Array.from(new Set([
    ...COMMON_COMPONENT_TYPES,
    ...reflectComponentTypes,
  ])).sort((a, b) => a.localeCompare(b)), [reflectComponentTypes]);
  const selectedEntityComponentTypes = new Set(selectedEntity?.components.map((component) => component.component_type) ?? []);
  const addableComponentTypes = availableComponentTypes.filter((componentType) => !selectedEntityComponentTypes.has(componentType));
  const terrainDetails = viewportModel.terrain;
  const hasGroundSurface = viewportModel.markers.some((marker) => marker.kind === "ground");
  const isDockedViewport = layoutMode === "dock";
  const viewportFocusEntity = useMemo(() => {
    const selectedMarker = viewportModel.markers.find((marker) => marker.id === selectedEntityId) ?? null;
    if (selectedMarker && selectedMarker.kind !== "light" && selectedMarker.kind !== "ground") {
      return selectedMarker;
    }
    const sortedMarkers = [...viewportModel.markers].sort((left, right) => viewportKindPriority(left.kind) - viewportKindPriority(right.kind));
    return sortedMarkers[0] ?? selectedMarker ?? null;
  }, [selectedEntityId, viewportModel.markers]);

  // Convert scene data to Viewport3D format for the WebGL renderer
  const viewportEntities: SceneEntity[] = useMemo(() => {
    const result: SceneEntity[] = [];
    for (const marker of viewportModel.markers) {
      const colorHex = marker.tone || "#7eb8d4";
      const r = parseInt(colorHex.slice(1, 3), 16) / 255;
      const g = parseInt(colorHex.slice(3, 5), 16) / 255;
      const b = parseInt(colorHex.slice(5, 7), 16) / 255;
      let kind: SceneEntity["kind"] = "cube";
      if (marker.kind === "light") kind = "light";
      else if (marker.kind === "camera") kind = "camera";
      else if (marker.kind === "ground") kind = "terrain";
      else if (marker.name.toLowerCase().includes("sphere") || marker.name.toLowerCase().includes("ball")) kind = "sphere";
      result.push({
        id: marker.id,
        name: marker.name,
        position: { x: marker.position[0], y: marker.position[1], z: marker.position[2] },
        scale: { x: marker.scale[0], y: marker.scale[1], z: marker.scale[2] },
        color: [r, g, b],
        kind,
      });
    }
    if (viewportModel.terrain) {
      result.push({
        id: viewportModel.terrain.entityId,
        name: viewportModel.terrain.entityName,
        position: { x: viewportModel.terrain.offset[0], y: viewportModel.terrain.offset[1], z: viewportModel.terrain.offset[2] },
        scale: { x: 1, y: 1, z: 1 },
        color: [0.3, 0.6, 0.3],
        kind: "terrain",
      });
    }
    return result;
  }, [viewportModel]);

  useEffect(() => {
    if (!viewportFocusEntity) {
      if (!scene) {
        setFirstPersonPosition(null);
      }
      return;
    }

    const focusForward = rotateVectorByQuaternion([0, 0, 1], viewportFocusEntity.rotation);
    const { yaw, pitch } = directionToYawPitch(focusForward);
    const eyeHeight = Math.max(1.35, Math.abs(viewportFocusEntity.scale[1]) * 0.75);
    const nextFirstPersonPosition: [number, number, number] = [
      viewportFocusEntity.position[0],
      viewportFocusEntity.position[1] + eyeHeight,
      viewportFocusEntity.position[2],
    ];

    if (!firstPersonPosition) {
      setFirstPersonPosition(nextFirstPersonPosition);
      setFirstPersonYaw(yaw);
      setFirstPersonPitch(pitch);
    }
  }, [firstPersonPosition, scene, viewportFocusEntity]);

  useEffect(() => {
    if (cameraMode !== "first_person" || !viewportFocusEntity) {
      return;
    }

    const focusForward = rotateVectorByQuaternion([0, 0, 1], viewportFocusEntity.rotation);
    const { yaw, pitch } = directionToYawPitch(focusForward);
    const eyeHeight = Math.max(1.35, Math.abs(viewportFocusEntity.scale[1]) * 0.75);
    setFirstPersonYaw(yaw);
    setFirstPersonPitch(pitch);
    setFirstPersonPosition([
      viewportFocusEntity.position[0],
      viewportFocusEntity.position[1] + eyeHeight,
      viewportFocusEntity.position[2],
    ]);
  }, [cameraMode, viewportFocusEntity]);

  useEffect(() => {
    if (!scene) {
      return;
    }
    const bounds = viewportModel.bounds;
    const span = Math.max(bounds.maxX - bounds.minX, bounds.maxZ - bounds.minZ, 6);
    setOrbitDistance(clamp(span * 1.15, 10, 38));
    setThirdPersonDistance(clamp(span * 0.42, 4, 14));
  }, [scene, viewportModel.bounds]);

  const resetViewportCamera = useCallback(() => {
    setOrbitYaw(-0.85);
    setOrbitPitch(0.48);
    setOrbitDistance(22);
    setThirdPersonYawOffset(0);
    setThirdPersonPitch(0.18);
    setThirdPersonDistance(7.5);
    if (viewportFocusEntity) {
      const focusForward = rotateVectorByQuaternion([0, 0, 1], viewportFocusEntity.rotation);
      const { yaw, pitch } = directionToYawPitch(focusForward);
      const eyeHeight = Math.max(1.35, Math.abs(viewportFocusEntity.scale[1]) * 0.75);
      setFirstPersonYaw(yaw);
      setFirstPersonPitch(pitch);
      setFirstPersonPosition([
        viewportFocusEntity.position[0],
        viewportFocusEntity.position[1] + eyeHeight,
        viewportFocusEntity.position[2],
      ]);
    }
  }, [viewportFocusEntity]);

  const handleViewportPointerDown = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }
    pointerDragRef.current = { x: event.clientX, y: event.clientY };
    setViewportFocused(true);
    viewportRef.current?.focus();
    event.preventDefault();
  }, []);

  const handleViewportWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (cameraMode === "orbit") {
      setOrbitDistance((current) => clamp(current + event.deltaY * 0.018, 5, 60));
      return;
    }
    if (cameraMode === "third_person") {
      setThirdPersonDistance((current) => clamp(current + event.deltaY * 0.015, 2.5, 20));
    }
  }, [cameraMode]);

  useEffect(() => {
    const handleMouseMove = (event: MouseEvent) => {
      const previous = pointerDragRef.current;
      if (!previous) {
        return;
      }

      const deltaX = event.clientX - previous.x;
      const deltaY = event.clientY - previous.y;
      pointerDragRef.current = { x: event.clientX, y: event.clientY };

      if (cameraMode === "orbit") {
        setOrbitYaw((current) => current + deltaX * 0.012);
        setOrbitPitch((current) => clamp(current - deltaY * 0.009, -1.2, 1.2));
      } else if (cameraMode === "third_person") {
        setThirdPersonYawOffset((current) => current + deltaX * 0.012);
        setThirdPersonPitch((current) => clamp(current - deltaY * 0.008, -0.55, 1.05));
      } else {
        setFirstPersonYaw((current) => current + deltaX * 0.012);
        setFirstPersonPitch((current) => clamp(current - deltaY * 0.008, -1.3, 1.3));
      }
    };

    const handleMouseUp = () => {
      pointerDragRef.current = null;
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [cameraMode]);

  useEffect(() => {
    if (!visible || cameraMode !== "first_person" || !viewportFocused) {
      keyStateRef.current.clear();
      return;
    }

    const supportedKeys = new Set(["w", "a", "s", "d", "q", "e", "arrowup", "arrowdown", "arrowleft", "arrowright", "shift"]);

    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if (!supportedKeys.has(key)) {
        return;
      }
      keyStateRef.current.add(key);
      event.preventDefault();
    };

    const handleKeyUp = (event: KeyboardEvent) => {
      keyStateRef.current.delete(event.key.toLowerCase());
    };

    let frame = 0;
    let lastTime = performance.now();
    const tick = (now: number) => {
      const deltaSeconds = Math.min(0.05, (now - lastTime) / 1000);
      lastTime = now;
      const keys = keyStateRef.current;
      if (keys.size > 0) {
        const forward = yawPitchToDirection(firstPersonYaw, 0);
        const right = vec3Normalize(vec3Cross(forward, [0, 1, 0]));
        let movement: [number, number, number] = [0, 0, 0];
        if (keys.has("w") || keys.has("arrowup")) movement = vec3Add(movement, forward);
        if (keys.has("s") || keys.has("arrowdown")) movement = vec3Subtract(movement, forward);
        if (keys.has("d") || keys.has("arrowright")) movement = vec3Add(movement, right);
        if (keys.has("a") || keys.has("arrowleft")) movement = vec3Subtract(movement, right);
        if (keys.has("e")) movement = vec3Add(movement, [0, 1, 0]);
        if (keys.has("q")) movement = vec3Subtract(movement, [0, 1, 0]);
        const normalizedMovement = vec3Length(movement) > 0 ? vec3Normalize(movement) : null;
        if (normalizedMovement) {
          const speed = keys.has("shift") ? 10 : 5.5;
          setFirstPersonPosition((current) => {
            const fallback = current ?? (viewportFocusEntity
              ? [viewportFocusEntity.position[0], viewportFocusEntity.position[1] + 1.6, viewportFocusEntity.position[2]]
              : [0, 1.6, 8]);
            return vec3Add(fallback, vec3Scale(normalizedMovement, speed * deltaSeconds));
          });
        }
      }
      frame = window.requestAnimationFrame(tick);
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    frame = window.requestAnimationFrame(tick);

    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
      keyStateRef.current.clear();
    };
  }, [cameraMode, firstPersonYaw, viewportFocused, viewportFocusEntity, visible]);

  useEffect(() => {
    if (!visible) return;
    const canvas = canvasRef.current;
    const viewport = viewportRef.current;
    if (!canvas || !viewport) return;

    const draw = () => {
      const width = Math.max(240, Math.floor(viewport.clientWidth));
      const height = Math.max(220, Math.floor(viewport.clientHeight));
      const dpr = window.devicePixelRatio || 1;
      const pixelWidth = Math.max(1, Math.floor(width * dpr));
      const pixelHeight = Math.max(1, Math.floor(height * dpr));

      if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
        canvas.width = pixelWidth;
        canvas.height = pixelHeight;
      }
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;

      const context = canvas.getContext("2d");
      if (!context) return;

      context.setTransform(1, 0, 0, 1, 0, 0);
      context.clearRect(0, 0, canvas.width, canvas.height);
      context.setTransform(dpr, 0, 0, dpr, 0, 0);

      const bounds = viewportModel.bounds;
      const spanX = Math.max(8, bounds.maxX - bounds.minX);
      const spanZ = Math.max(8, bounds.maxZ - bounds.minZ);
      const centerX = (bounds.minX + bounds.maxX) / 2;
      const centerY = (bounds.minY + bounds.maxY) / 2;
      const centerZ = (bounds.minZ + bounds.maxZ) / 2;
      const fallbackFocus: [number, number, number] = [centerX, centerY, centerZ];
      const focusPoint = viewportFocusEntity
        ? [
            viewportFocusEntity.position[0],
            viewportFocusEntity.position[1] + Math.max(0.5, Math.abs(viewportFocusEntity.scale[1]) * 0.3),
            viewportFocusEntity.position[2],
          ] as [number, number, number]
        : fallbackFocus;

      let cameraPosition: [number, number, number];
      let cameraTarget: [number, number, number];

      if (cameraMode === "first_person") {
        const origin = firstPersonPosition ?? vec3Add(focusPoint, [0, 1.6, Math.max(spanX, spanZ) * 0.25]);
        cameraPosition = origin;
        cameraTarget = vec3Add(origin, yawPitchToDirection(firstPersonYaw, firstPersonPitch));
      } else if (cameraMode === "third_person") {
        const forward = viewportFocusEntity
          ? rotateVectorByQuaternion([0, 0, 1], viewportFocusEntity.rotation)
          : yawPitchToDirection(orbitYaw, 0);
        const baseAngles = directionToYawPitch(forward);
        const lookDirection = yawPitchToDirection(baseAngles.yaw + thirdPersonYawOffset, clamp(baseAngles.pitch + thirdPersonPitch, -0.7, 1.1));
        const anchor = viewportFocusEntity
          ? [
              viewportFocusEntity.position[0],
              viewportFocusEntity.position[1] + Math.max(0.8, Math.abs(viewportFocusEntity.scale[1]) * 0.7),
              viewportFocusEntity.position[2],
            ] as [number, number, number]
          : focusPoint;
        cameraTarget = vec3Add(anchor, vec3Scale(lookDirection, 2));
        cameraPosition = vec3Add(anchor, vec3Add(
          vec3Scale(lookDirection, -thirdPersonDistance),
          [0, Math.max(1.1, thirdPersonDistance * 0.18), 0] as [number, number, number],
        ));
      } else {
        const orbitDirection = yawPitchToDirection(orbitYaw, orbitPitch);
        cameraTarget = focusPoint;
        cameraPosition = vec3Add(focusPoint, vec3Scale(orbitDirection, orbitDistance));
      }

      const cameraForward = vec3Normalize(vec3Subtract(cameraTarget, cameraPosition));
      const cameraRight = vec3Normalize(vec3Cross(cameraForward, [0, 1, 0]));
      const safeRight: [number, number, number] = vec3Length(cameraRight) > 0.001 ? cameraRight : [1, 0, 0];
      const cameraUp = vec3Normalize(vec3Cross(safeRight, cameraForward));
      const fieldOfView = (cameraMode === "first_person" ? 80 : 62) * Math.PI / 180;
      const focalLength = height / (2 * Math.tan(fieldOfView / 2));
      const lightVector = vec3Normalize(vec3Scale(viewportModel.lightDirection, -1));

      interface ProjectedPoint extends Vec2Point {
        depth: number;
      }

      interface RenderPolygon {
        depth: number;
        points: ProjectedPoint[];
        fill: string;
        stroke: string;
        lineWidth: number;
      }

      interface GroundSurface {
        marker: ViewportMarker;
        topCorners: Array<[number, number, number]>;
        topY: number;
      }

      interface LightGizmo {
        marker: ViewportMarker;
        start: [number, number, number];
        end: [number, number, number];
      }

      const projectDirection = (direction: [number, number, number]): ProjectedPoint | null => {
        const normalizedDirection = vec3Normalize(direction);
        const depth = vec3Dot(normalizedDirection, cameraForward);
        if (depth <= 0.02) {
          return null;
        }
        return {
          x: width / 2 + (vec3Dot(normalizedDirection, safeRight) / depth) * focalLength,
          y: height / 2 - (vec3Dot(normalizedDirection, cameraUp) / depth) * focalLength,
          depth,
        };
      };

      const dayStrength = smoothstep(-0.16, 0.22, lightVector[1]);
      const twilightStrength = 1 - smoothstep(0.12, 0.5, Math.abs(lightVector[1]));
      const nightStrength = 1 - dayStrength;
      const skyTop = mixColor(mixColor("#050912", "#4d87cb", dayStrength), "#7e506f", twilightStrength * 0.5);
      const skyMid = mixColor(mixColor("#0d1623", "#7ca8dc", dayStrength), "#cf7d58", twilightStrength * 0.42);
      const skyHorizon = mixColor(mixColor("#17202d", "#dbeafb", dayStrength), "#f0a66f", twilightStrength * 0.72);
      const skyGradient = context.createLinearGradient(0, 0, 0, height);
      skyGradient.addColorStop(0, skyTop);
      skyGradient.addColorStop(0.52, skyMid);
      skyGradient.addColorStop(1, skyHorizon);
      context.fillStyle = skyGradient;
      context.fillRect(0, 0, width, height);

      const horizonGlow = context.createLinearGradient(0, height * 0.34, 0, height);
      horizonGlow.addColorStop(0, "rgba(255, 255, 255, 0)");
      horizonGlow.addColorStop(0.46, mixColor("#6f8fb0", "#ffbf88", twilightStrength * 0.78, 0.08 + dayStrength * 0.06));
      horizonGlow.addColorStop(1, mixColor("#193047", "#f3c590", twilightStrength * 0.82, 0.18 + dayStrength * 0.08));
      context.fillStyle = horizonGlow;
      context.fillRect(0, 0, width, height);

      const starSeeds = [
        [-2.75, 0.88, 0.7], [-2.3, 0.72, 1], [-1.92, 0.95, 0.9], [-1.4, 0.66, 0.7],
        [-0.88, 0.84, 1.1], [-0.34, 0.58, 0.8], [0.12, 0.9, 0.9], [0.56, 0.68, 0.7],
        [0.92, 0.82, 1.15], [1.26, 0.62, 0.85], [1.72, 0.78, 0.72], [2.15, 0.9, 1],
        [2.48, 0.7, 0.84], [2.82, 0.56, 0.7], [-2.02, 0.46, 0.74], [1.98, 0.5, 0.76],
      ] as const;
      if (nightStrength > 0.1) {
        for (const [yaw, pitch, radius] of starSeeds) {
          const star = projectDirection(yawPitchToDirection(yaw, pitch));
          if (!star || star.y > height * 0.9) {
            continue;
          }
          const glow = context.createRadialGradient(star.x, star.y, 0, star.x, star.y, 6 * radius);
          glow.addColorStop(0, `rgba(255, 255, 255, ${0.55 * nightStrength})`);
          glow.addColorStop(1, "rgba(255, 255, 255, 0)");
          context.fillStyle = glow;
          context.beginPath();
          context.arc(star.x, star.y, 6 * radius, 0, Math.PI * 2);
          context.fill();

          context.fillStyle = `rgba(255, 251, 238, ${0.42 + nightStrength * 0.42})`;
          context.beginPath();
          context.arc(star.x, star.y, radius, 0, Math.PI * 2);
          context.fill();
        }
      }

      const sunProjection = projectDirection(lightVector);
      const moonProjection = projectDirection(vec3Normalize(vec3Add(vec3Scale(lightVector, -1), [0.08, 0.14, -0.03])));

      const drawEllipticalGradient = (
        centerX: number,
        centerY: number,
        radiusX: number,
        radiusY: number,
        stops: Array<[number, string]>,
        rotation = 0,
      ) => {
        if (radiusX <= 0 || radiusY <= 0) {
          return;
        }
        context.save();
        context.translate(centerX, centerY);
        if (rotation !== 0) {
          context.rotate(rotation);
        }
        context.scale(radiusX, radiusY);
        const gradient = context.createRadialGradient(-0.2, -0.28, 0.12, 0, 0, 1);
        for (const [offset, color] of stops) {
          gradient.addColorStop(clamp(offset, 0, 1), color);
        }
        context.fillStyle = gradient;
        context.beginPath();
        context.arc(0, 0, 1, 0, Math.PI * 2);
        context.fill();
        context.restore();
      };

      const drawCloudCluster = (
        centerX: number,
        centerY: number,
        widthScale: number,
        heightScale: number,
        tilt: number,
        seedBase: number,
        sunTint: number,
        heightFactor: number,
        alphaBase: number,
      ) => {
        const sunSide = sunProjection ? Math.sign(sunProjection.x - centerX) || 1 : 1;
        const lightLift = clamp(0.28 + sunTint * 0.9 + dayStrength * 0.18, 0.2, 1);
        const coolLift = clamp(0.22 + nightStrength * 0.28, 0.15, 0.58);
        const bodyAlpha = clamp(alphaBase + heightFactor * 0.05, 0.12, 0.4);
        const shadowAlpha = clamp(0.09 + heightFactor * 0.1 + nightStrength * 0.06, 0.08, 0.3);
        const hazeAlpha = clamp(bodyAlpha * 0.32, 0.08, 0.18);

        context.save();
        context.translate(centerX, centerY);
        context.rotate(tilt);

        drawEllipticalGradient(
          0,
          heightScale * 0.28,
          widthScale * 0.96,
          heightScale * 0.72,
          [
            [0, mixColor("#2a3746", "#6e8196", dayStrength * 0.44, shadowAlpha)],
            [0.5, mixColor("#22303e", "#4f6276", dayStrength * 0.36, shadowAlpha * 0.78)],
            [1, "rgba(12, 17, 24, 0)"],
          ],
          0,
        );

        drawEllipticalGradient(
          0,
          0,
          widthScale * 1.08,
          heightScale * 0.86,
          [
            [0, mixColor("#d9e5ef", "#ffffff", lightLift * 0.62, hazeAlpha)],
            [0.58, mixColor("#bfd0df", "#f7fbff", lightLift * 0.45, hazeAlpha * 0.8)],
            [1, "rgba(255, 255, 255, 0)"],
          ],
          0,
        );

        const puffCount = 8;
        for (let index = 0; index < puffCount; index += 1) {
          const localSeed = seedBase * 19.17 + index * 7.31;
          const xBias = lerp(-0.52, 0.5, index / Math.max(1, puffCount - 1));
          const xJitter = seededSigned(localSeed + 0.2) * 0.11;
          const yJitter = seededSigned(localSeed + 0.6) * 0.11;
          const rx = widthScale * lerp(0.2, 0.36, seededUnit(localSeed + 1.1));
          const ry = heightScale * lerp(0.22, 0.4, seededUnit(localSeed + 1.7));
          const puffX = widthScale * (xBias + xJitter);
          const puffY = heightScale * (lerp(-0.18, 0.1, seededUnit(localSeed + 2.3)) + yJitter);
          const localLight = clamp(lightLift + sunSide * (puffX / Math.max(1, widthScale)) * 0.2, 0.18, 1);
          const topShade = mixColor("#bccddb", "#ffffff", localLight * 0.78 + sunTint * 0.18, bodyAlpha);
          const midShade = mixColor("#8da1b6", "#edf5fb", localLight * 0.55 + coolLift * 0.18, bodyAlpha * 0.84);
          drawEllipticalGradient(
            puffX,
            puffY,
            rx,
            ry,
            [
              [0, topShade],
              [0.56, midShade],
              [1, "rgba(255, 255, 255, 0)"],
            ],
            seededSigned(localSeed + 2.8) * 0.12,
          );

          drawEllipticalGradient(
            puffX,
            puffY + ry * 0.34,
            rx * 0.92,
            ry * 0.72,
            [
              [0, mixColor("#425260", "#74889d", dayStrength * 0.4, shadowAlpha * 0.42)],
              [1, "rgba(32, 42, 52, 0)"],
            ],
            seededSigned(localSeed + 3.2) * 0.08,
          );
        }

        const highlightAnchorX = widthScale * 0.42 * sunSide;
        const highlightAnchorY = -heightScale * 0.16;
        drawEllipticalGradient(
          highlightAnchorX,
          highlightAnchorY,
          widthScale * 0.44,
          heightScale * 0.28,
          [
            [0, mixColor("#ffd79e", "#ffffff", dayStrength * 0.74 + sunTint * 0.16, 0.18 + sunTint * 0.14)],
            [0.55, mixColor("#ffd0a5", "#fff3d2", dayStrength * 0.52 + twilightStrength * 0.2, 0.08 + sunTint * 0.08)],
            [1, "rgba(255, 255, 255, 0)"],
          ],
          -tilt * 0.4,
        );

        for (let index = 0; index < 5; index += 1) {
          const localSeed = seedBase * 13.9 + index * 4.1;
          const wispY = heightScale * lerp(0.12, 0.38, seededUnit(localSeed + 0.3));
          const wispWidth = widthScale * lerp(0.5, 0.92, seededUnit(localSeed + 0.6));
          const wispHeight = heightScale * lerp(0.08, 0.16, seededUnit(localSeed + 0.9));
          drawEllipticalGradient(
            widthScale * seededSigned(localSeed + 1.2) * 0.18,
            wispY,
            wispWidth,
            wispHeight,
            [
              [0, mixColor("#d9e7f2", "#ffffff", 0.4 + dayStrength * 0.26, 0.045 + bodyAlpha * 0.12)],
              [1, "rgba(255, 255, 255, 0)"],
            ],
            seededSigned(localSeed + 1.6) * 0.1,
          );
        }

        context.restore();
      };

      if (moonProjection) {
        const moonRadius = lerp(12, 18, nightStrength);
        const moonGlow = context.createRadialGradient(moonProjection.x, moonProjection.y, 0, moonProjection.x, moonProjection.y, moonRadius * 4.4);
        moonGlow.addColorStop(0, `rgba(212, 228, 255, ${0.18 + nightStrength * 0.18})`);
        moonGlow.addColorStop(0.45, `rgba(182, 209, 255, ${0.08 + nightStrength * 0.1})`);
        moonGlow.addColorStop(1, "rgba(182, 209, 255, 0)");
        context.fillStyle = moonGlow;
        context.fillRect(0, 0, width, height);

        context.fillStyle = `rgba(233, 240, 255, ${0.18 + nightStrength * 0.66})`;
        context.beginPath();
        context.arc(moonProjection.x, moonProjection.y, moonRadius, 0, Math.PI * 2);
        context.fill();
        context.fillStyle = mixColor("#101a28", "#324760", dayStrength * 0.35, 0.82);
        context.beginPath();
        context.arc(moonProjection.x + moonRadius * 0.38, moonProjection.y - moonRadius * 0.06, moonRadius * 0.92, 0, Math.PI * 2);
        context.fill();
      }

      if (sunProjection) {
        const sunRadius = lerp(18, 28, dayStrength + twilightStrength * 0.25);
        const sunGlow = context.createRadialGradient(sunProjection.x, sunProjection.y, 0, sunProjection.x, sunProjection.y, sunRadius * 6);
        sunGlow.addColorStop(0, `rgba(255, 241, 198, ${0.22 + dayStrength * 0.48})`);
        sunGlow.addColorStop(0.3, `rgba(255, 202, 108, ${0.12 + twilightStrength * 0.12})`);
        sunGlow.addColorStop(1, "rgba(255, 198, 113, 0)");
        context.fillStyle = sunGlow;
        context.fillRect(0, 0, width, height);

        context.fillStyle = mixColor("#ffd998", "#fff2c6", dayStrength * 0.65 + twilightStrength * 0.25, 0.92);
        context.beginPath();
        context.arc(sunProjection.x, sunProjection.y, sunRadius, 0, Math.PI * 2);
        context.fill();
      }

      const cloudSeeds = [
        { yaw: -2.62, pitch: 0.34, scale: 1.15, tilt: 0.08 },
        { yaw: -1.88, pitch: 0.27, scale: 0.96, tilt: -0.04 },
        { yaw: -1.16, pitch: 0.4, scale: 1.2, tilt: 0.12 },
        { yaw: -0.18, pitch: 0.3, scale: 0.9, tilt: -0.08 },
        { yaw: 0.62, pitch: 0.38, scale: 1.12, tilt: 0.06 },
        { yaw: 1.36, pitch: 0.24, scale: 1.24, tilt: -0.05 },
        { yaw: 2.08, pitch: 0.35, scale: 0.94, tilt: 0.04 },
        { yaw: 2.72, pitch: 0.29, scale: 1.05, tilt: -0.09 },
      ] as const;
      const cloudAlphaBase = 0.08 + dayStrength * 0.18 + twilightStrength * 0.08;
      for (const [index, seed] of cloudSeeds.entries()) {
        const cloud = projectDirection(yawPitchToDirection(seed.yaw, seed.pitch));
        if (!cloud || cloud.y > height * 0.9) {
          continue;
        }
        const heightFactor = 1 - clamp(cloud.y / (height * 0.92), 0, 1);
        const widthScale = seed.scale * lerp(78, 176, heightFactor);
        const heightScale = widthScale * lerp(0.26, 0.4, heightFactor);
        const sunDistance = sunProjection ? Math.hypot(cloud.x - sunProjection.x, cloud.y - sunProjection.y) / Math.max(width, height) : 1;
        const sunTint = clamp(1 - sunDistance * 1.7, 0, 1);
        drawCloudCluster(
          cloud.x,
          cloud.y,
          widthScale,
          heightScale,
          seed.tilt,
          index + 1,
          sunTint,
          heightFactor,
          cloudAlphaBase,
        );
      }

      const skyVignette = context.createRadialGradient(width * 0.5, height * 0.38, height * 0.12, width * 0.5, height * 0.38, height * 0.94);
      skyVignette.addColorStop(0, "rgba(0, 0, 0, 0)");
      skyVignette.addColorStop(1, "rgba(3, 7, 12, 0.3)");
      context.fillStyle = skyVignette;
      context.fillRect(0, 0, width, height);

      const projectPoint = (point: [number, number, number]): ProjectedPoint | null => {
        const relative = vec3Subtract(point, cameraPosition);
        const depth = vec3Dot(relative, cameraForward);
        if (depth <= 0.08) {
          return null;
        }
        return {
          x: width / 2 + (vec3Dot(relative, safeRight) / depth) * focalLength,
          y: height / 2 - (vec3Dot(relative, cameraUp) / depth) * focalLength,
          depth,
        };
      };

      const polygons: RenderPolygon[] = [];
      const groundSurfaces: GroundSurface[] = [];
      const lightGizmos: LightGizmo[] = [];
      const addPolygon = (
        worldPoints: Array<[number, number, number]>,
        baseColor: string,
        alpha = 0.95,
        lineWidth = 1,
      ) => {
        if (worldPoints.length < 3) {
          return;
        }
        const projected = worldPoints.map(projectPoint);
        if (projected.some((point) => !point)) {
          return;
        }
        const center = vec3Average(worldPoints);
        const normal = vec3Normalize(vec3Cross(
          vec3Subtract(worldPoints[1], worldPoints[0]),
          vec3Subtract(worldPoints[2], worldPoints[0]),
        ));
        const facing = vec3Dot(normal, vec3Subtract(cameraPosition, center));
        if (facing <= 0) {
          return;
        }
        const diffuse = clamp(0.18 + Math.max(0, vec3Dot(normal, lightVector)) * 0.82, 0.16, 1.2);
        polygons.push({
          depth: projected.reduce((sum, point) => sum + (point?.depth ?? 0), 0) / projected.length,
          points: projected as ProjectedPoint[],
          fill: shadeColor(baseColor, diffuse, alpha),
          stroke: shadeColor(baseColor, diffuse * 0.74 + 0.1, Math.min(0.58, alpha)),
          lineWidth,
        });
      };

      const addGroundPlane = (marker: ViewportMarker) => {
        const halfScale: [number, number, number] = [
          Math.max(2.4, Math.abs(marker.scale[0]) * 0.5),
          Math.max(0.12, Math.abs(marker.scale[1]) * 0.5),
          Math.max(2.4, Math.abs(marker.scale[2]) * 0.5),
        ];
        const localTop: Array<[number, number, number]> = [
          [-halfScale[0], halfScale[1], -halfScale[2]],
          [-halfScale[0], halfScale[1], halfScale[2]],
          [halfScale[0], halfScale[1], halfScale[2]],
          [halfScale[0], halfScale[1], -halfScale[2]],
        ];
        const localBottom: Array<[number, number, number]> = [
          [-halfScale[0], -halfScale[1], -halfScale[2]],
          [-halfScale[0], -halfScale[1], halfScale[2]],
          [halfScale[0], -halfScale[1], halfScale[2]],
          [halfScale[0], -halfScale[1], -halfScale[2]],
        ];
        const topCorners = localTop.map((corner) => vec3Add(marker.position, rotateVectorByQuaternion(corner, marker.rotation)));
        const bottomCorners = localBottom.map((corner) => vec3Add(marker.position, rotateVectorByQuaternion(corner, marker.rotation)));
        addPolygon(topCorners, marker.selected ? "#8ea37d" : "#677b61", 0.98, 0.35);
        addPolygon([bottomCorners[0], bottomCorners[1], topCorners[1], topCorners[0]], "#425046", 0.92, 0.28);
        addPolygon([bottomCorners[1], bottomCorners[2], topCorners[2], topCorners[1]], "#39443e", 0.92, 0.28);
        addPolygon([bottomCorners[2], bottomCorners[3], topCorners[3], topCorners[2]], "#33403a", 0.92, 0.28);
        addPolygon([bottomCorners[3], bottomCorners[0], topCorners[0], topCorners[3]], "#445248", 0.92, 0.28);
        groundSurfaces.push({
          marker,
          topCorners,
          topY: vec3Average(topCorners)[1],
        });
      };

      const addBox = (marker: ViewportMarker) => {
        if (marker.kind === "ground") {
          addGroundPlane(marker);
          return;
        }
        if (marker.kind === "light") {
          const lightStart = vec3Add(marker.position, [0, Math.max(0.8, Math.abs(marker.scale[1]) * 0.8) + 0.35, 0]);
          lightGizmos.push({
            marker,
            start: lightStart,
            end: vec3Add(lightStart, vec3Scale(viewportModel.lightDirection, 3.2)),
          });
          return;
        }
        const halfScale: [number, number, number] = [
          Math.max(0.2, Math.abs(marker.scale[0]) * 0.5),
          Math.max(0.25, Math.abs(marker.scale[1]) * 0.5),
          Math.max(0.2, Math.abs(marker.scale[2]) * 0.5),
        ];
        const localCorners: Array<[number, number, number]> = [
          [-halfScale[0], -halfScale[1], -halfScale[2]],
          [halfScale[0], -halfScale[1], -halfScale[2]],
          [halfScale[0], halfScale[1], -halfScale[2]],
          [-halfScale[0], halfScale[1], -halfScale[2]],
          [-halfScale[0], -halfScale[1], halfScale[2]],
          [halfScale[0], -halfScale[1], halfScale[2]],
          [halfScale[0], halfScale[1], halfScale[2]],
          [-halfScale[0], halfScale[1], halfScale[2]],
        ];
        const corners = localCorners.map((corner) => vec3Add(marker.position, rotateVectorByQuaternion(corner, marker.rotation)));
        const faces = [
          [0, 1, 2, 3],
          [4, 5, 6, 7],
          [0, 4, 7, 3],
          [1, 5, 6, 2],
          [3, 2, 6, 7],
          [0, 1, 5, 4],
        ] as const;
        const baseColor = marker.selected ? "#f0b96e" : marker.tone;
        for (const face of faces) {
          addPolygon(face.map((index) => corners[index]), baseColor, marker.selected ? 0.98 : 0.9, marker.selected ? 1.15 : 0.9);
        }
      };

      if (terrainDetails) {
        const cols = Math.min(terrainDetails.cols, isDockedViewport ? 34 : 42);
        const rows = Math.min(terrainDetails.rows, isDockedViewport ? 34 : 42);
        const terrainSample = (col: number, row: number): [number, number, number] => {
          const localX = (col / Math.max(1, cols - 1) - 0.5) * terrainDetails.scale;
          const localZ = (row / Math.max(1, rows - 1) - 0.5) * terrainDetails.scale;
          return [
            terrainDetails.offset[0] + localX,
            terrainDetails.offset[1] + terrainHeightAt(localX, localZ, terrainDetails.heightScale, terrainDetails.frequency),
            terrainDetails.offset[2] + localZ,
          ];
        };

        for (let row = 0; row < rows - 1; row += 1) {
          for (let col = 0; col < cols - 1; col += 1) {
            const p00 = terrainSample(col, row);
            const p10 = terrainSample(col + 1, row);
            const p01 = terrainSample(col, row + 1);
            const p11 = terrainSample(col + 1, row + 1);
            const averageHeight = (p00[1] + p10[1] + p01[1] + p11[1]) / 4;
            const normalizedHeight = clamp((averageHeight - bounds.minY) / Math.max(1.5, bounds.maxY - bounds.minY), 0, 1);
            const baseColor = normalizedHeight > 0.68 ? "#a5bf8f" : normalizedHeight > 0.38 ? "#5e9772" : "#4c6e7f";
            addPolygon([p00, p01, p11, p10], baseColor, 0.96, 0.45);
          }
        }
      }

      for (const marker of viewportModel.markers) {
        addBox(marker);
      }

      polygons.sort((left, right) => right.depth - left.depth);
      for (const polygon of polygons) {
        context.beginPath();
        polygon.points.forEach((point, index) => {
          if (index === 0) {
            context.moveTo(point.x, point.y);
          } else {
            context.lineTo(point.x, point.y);
          }
        });
        context.closePath();
        context.fillStyle = polygon.fill;
        context.fill();
        context.strokeStyle = polygon.stroke;
        context.lineWidth = polygon.lineWidth;
        context.stroke();
      }

      if (viewportModel.markers.length > 0 || terrainDetails) {
        const gridCenterX = terrainDetails?.offset[0] ?? centerX;
        const gridCenterZ = terrainDetails?.offset[2] ?? centerZ;
        const gridY = terrainDetails
          ? terrainDetails.offset[1]
          : groundSurfaces[0]?.topY ?? bounds.minY;
        const gridHalfExtent = Math.max(18, Math.max(spanX, spanZ) * 0.85);
        const gridStep = gridHalfExtent > 42 ? 4 : 2;
        const gridStart = Math.floor(-gridHalfExtent / gridStep) * gridStep;
        const gridEnd = Math.ceil(gridHalfExtent / gridStep) * gridStep;

        for (let offset = gridStart; offset <= gridEnd; offset += gridStep) {
          const major = offset === 0 || Math.abs(offset) % (gridStep * 4) === 0;
          const xColor = major ? "rgba(148, 196, 220, 0.16)" : "rgba(148, 196, 220, 0.08)";
          const zColor = major ? "rgba(148, 196, 220, 0.14)" : "rgba(148, 196, 220, 0.06)";

          const xStart = projectPoint([gridCenterX + offset, gridY + 0.02, gridCenterZ - gridHalfExtent]);
          const xEnd = projectPoint([gridCenterX + offset, gridY + 0.02, gridCenterZ + gridHalfExtent]);
          if (xStart && xEnd) {
            context.beginPath();
            context.strokeStyle = xColor;
            context.lineWidth = major ? 1.05 : 0.8;
            context.moveTo(xStart.x, xStart.y);
            context.lineTo(xEnd.x, xEnd.y);
            context.stroke();
          }

          const zStart = projectPoint([gridCenterX - gridHalfExtent, gridY + 0.02, gridCenterZ + offset]);
          const zEnd = projectPoint([gridCenterX + gridHalfExtent, gridY + 0.02, gridCenterZ + offset]);
          if (zStart && zEnd) {
            context.beginPath();
            context.strokeStyle = zColor;
            context.lineWidth = major ? 1.05 : 0.8;
            context.moveTo(zStart.x, zStart.y);
            context.lineTo(zEnd.x, zEnd.y);
            context.stroke();
          }
        }
      }

      if (groundSurfaces.length > 0) {
        const primaryGround = groundSurfaces[0];
        for (let index = 1; index <= 8; index += 1) {
          const t = index / 9;
          const startA = vec3Add(primaryGround.topCorners[0], vec3Scale(vec3Subtract(primaryGround.topCorners[3], primaryGround.topCorners[0]), t));
          const endA = vec3Add(primaryGround.topCorners[1], vec3Scale(vec3Subtract(primaryGround.topCorners[2], primaryGround.topCorners[1]), t));
          const startB = vec3Add(primaryGround.topCorners[0], vec3Scale(vec3Subtract(primaryGround.topCorners[1], primaryGround.topCorners[0]), t));
          const endB = vec3Add(primaryGround.topCorners[3], vec3Scale(vec3Subtract(primaryGround.topCorners[2], primaryGround.topCorners[3]), t));
          const projectedStartA = projectPoint(startA);
          const projectedEndA = projectPoint(endA);
          const projectedStartB = projectPoint(startB);
          const projectedEndB = projectPoint(endB);
          if (projectedStartA && projectedEndA) {
            context.beginPath();
            context.strokeStyle = "rgba(205, 226, 192, 0.1)";
            context.lineWidth = 1;
            context.moveTo(projectedStartA.x, projectedStartA.y);
            context.lineTo(projectedEndA.x, projectedEndA.y);
            context.stroke();
          }
          if (projectedStartB && projectedEndB) {
            context.beginPath();
            context.strokeStyle = "rgba(205, 226, 192, 0.08)";
            context.lineWidth = 1;
            context.moveTo(projectedStartB.x, projectedStartB.y);
            context.lineTo(projectedEndB.x, projectedEndB.y);
            context.stroke();
          }
        }

        const groundPlaneY = primaryGround.topY;
        const shadowDirection = vec3Normalize(viewportModel.lightDirection[1] > -0.12
          ? [viewportModel.lightDirection[0], -0.35, viewportModel.lightDirection[2]]
          : viewportModel.lightDirection);
        for (const marker of viewportModel.markers) {
          if (marker.kind === "ground" || marker.kind === "light") {
            continue;
          }
          const sourceHeight = Math.max(0.9, Math.abs(marker.scale[1]) * 0.8);
          const sourcePoint: [number, number, number] = [
            marker.position[0],
            marker.position[1] + sourceHeight,
            marker.position[2],
          ];
          const drop = Math.max(0.3, sourcePoint[1] - groundPlaneY);
          const directionScale = drop / Math.max(0.15, -shadowDirection[1]);
          const shadowCenter = vec3Add(sourcePoint, vec3Scale(shadowDirection, directionScale));
          const shadowRight = vec3Scale(safeRight, Math.max(0.4, Math.abs(marker.scale[0]) * 0.45));
          const shadowForward = vec3Scale(vec3Normalize(vec3Cross([0, 1, 0], safeRight)), Math.max(0.4, Math.abs(marker.scale[2]) * 0.45));
          const shadowPoints: Array<[number, number, number]> = [
            vec3Add(vec3Add(shadowCenter, shadowRight), shadowForward),
            vec3Add(vec3Subtract(shadowCenter, shadowRight), shadowForward),
            vec3Subtract(vec3Subtract(shadowCenter, shadowRight), shadowForward),
            vec3Subtract(vec3Add(shadowCenter, shadowRight), shadowForward),
          ].map((point) => [point[0], groundPlaneY + 0.02, point[2]]);
          const projectedShadow = shadowPoints.map(projectPoint);
          if (projectedShadow.some((point) => !point)) {
            continue;
          }
          context.beginPath();
          (projectedShadow as ProjectedPoint[]).forEach((point, index) => {
            if (index === 0) {
              context.moveTo(point.x, point.y);
            } else {
              context.lineTo(point.x, point.y);
            }
          });
          context.closePath();
          context.fillStyle = marker.selected ? "rgba(233, 170, 95, 0.12)" : "rgba(0, 0, 0, 0.12)";
          context.fill();
        }
      }

      for (const gizmo of lightGizmos) {
        const projectedStart = projectPoint(gizmo.start);
        const projectedEnd = projectPoint(gizmo.end);
        if (!projectedStart || !projectedEnd) {
          continue;
        }
        const radius = gizmo.marker.selected ? 9 : 7;
        context.beginPath();
        context.strokeStyle = gizmo.marker.selected ? "rgba(255, 222, 150, 0.98)" : "rgba(255, 214, 124, 0.88)";
        context.lineWidth = gizmo.marker.selected ? 2.8 : 2.2;
        context.moveTo(projectedStart.x, projectedStart.y);
        context.lineTo(projectedEnd.x, projectedEnd.y);
        context.stroke();

        const glow = context.createRadialGradient(projectedStart.x, projectedStart.y, 0, projectedStart.x, projectedStart.y, radius * 2.4);
        glow.addColorStop(0, gizmo.marker.selected ? "rgba(255, 228, 162, 0.95)" : "rgba(255, 214, 124, 0.9)");
        glow.addColorStop(0.45, gizmo.marker.selected ? "rgba(255, 198, 113, 0.34)" : "rgba(255, 198, 113, 0.24)");
        glow.addColorStop(1, "rgba(255, 198, 113, 0)");
        context.fillStyle = glow;
        context.beginPath();
        context.arc(projectedStart.x, projectedStart.y, radius * 2.4, 0, Math.PI * 2);
        context.fill();

        context.fillStyle = gizmo.marker.selected ? "#ffe0ac" : "#ffd67c";
        context.beginPath();
        context.arc(projectedStart.x, projectedStart.y, radius, 0, Math.PI * 2);
        context.fill();
        context.strokeStyle = "rgba(72, 40, 12, 0.45)";
        context.lineWidth = 1;
        context.stroke();

        const arrowVector = {
          x: projectedEnd.x - projectedStart.x,
          y: projectedEnd.y - projectedStart.y,
        };
        const arrowLength = Math.max(12, Math.hypot(arrowVector.x, arrowVector.y));
        const arrowUnit = {
          x: arrowVector.x / arrowLength,
          y: arrowVector.y / arrowLength,
        };
        const arrowLeft = {
          x: projectedEnd.x - arrowUnit.x * 12 - arrowUnit.y * 5,
          y: projectedEnd.y - arrowUnit.y * 12 + arrowUnit.x * 5,
        };
        const arrowRight = {
          x: projectedEnd.x - arrowUnit.x * 12 + arrowUnit.y * 5,
          y: projectedEnd.y - arrowUnit.y * 12 - arrowUnit.x * 5,
        };
        context.beginPath();
        context.fillStyle = gizmo.marker.selected ? "rgba(255, 222, 150, 0.98)" : "rgba(255, 214, 124, 0.88)";
        context.moveTo(projectedEnd.x, projectedEnd.y);
        context.lineTo(arrowLeft.x, arrowLeft.y);
        context.lineTo(arrowRight.x, arrowRight.y);
        context.closePath();
        context.fill();
      }

      const labels = [...viewportModel.markers].sort((left, right) => {
        const leftDistance = vec3Length(vec3Subtract(left.position, cameraPosition));
        const rightDistance = vec3Length(vec3Subtract(right.position, cameraPosition));
        return rightDistance - leftDistance;
      });
      for (const marker of labels) {
        const labelPoint = projectPoint([
          marker.position[0],
          marker.position[1] + Math.max(0.9, Math.abs(marker.scale[1]) * 0.7) + 0.35,
          marker.position[2],
        ]);
        if (!labelPoint) {
          continue;
        }
        context.font = marker.selected ? "700 11px system-ui" : "600 10px system-ui";
        context.textAlign = "center";
        context.fillStyle = marker.selected ? "#ffd39a" : "rgba(222, 232, 240, 0.9)";
        context.fillText(marker.name, labelPoint.x, labelPoint.y);
      }

      if (cameraMode === "first_person") {
        context.strokeStyle = "rgba(255, 255, 255, 0.65)";
        context.lineWidth = 1.2;
        context.beginPath();
        context.moveTo(width / 2 - 10, height / 2);
        context.lineTo(width / 2 + 10, height / 2);
        context.moveTo(width / 2, height / 2 - 10);
        context.lineTo(width / 2, height / 2 + 10);
        context.stroke();
      }

      if (!terrainDetails && viewportModel.markers.length === 0) {
        context.textAlign = "center";
        context.font = "600 14px system-ui";
        context.fillStyle = "rgba(216, 226, 231, 0.8)";
        context.fillText("No terrain or transform data yet", width / 2, height / 2 - 8);
        context.font = "12px system-ui";
        context.fillStyle = "rgba(142, 181, 196, 0.86)";
        context.fillText("Add scene entities or a Terrain component to render the 3D viewport.", width / 2, height / 2 + 14);
      }
    };

    draw();
    const observer = new ResizeObserver(() => draw());
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [
    cameraMode,
    firstPersonPitch,
    firstPersonPosition,
    firstPersonYaw,
    isDockedViewport,
    orbitDistance,
    orbitPitch,
    orbitYaw,
    terrainDetails,
    thirdPersonDistance,
    thirdPersonPitch,
    thirdPersonYawOffset,
    viewportFocusEntity,
    viewportModel,
    visible,
  ]);

  const openProjectFile = useCallback((relativePath: string) => {
    if (!projectPath || !relativePath) return;
    const path = resolveProjectPath(projectPath, relativePath);
    const name = relativePath.split("/").pop() || path.split("/").pop() || relativePath;
    onOpenFile?.(path, name);
  }, [onOpenFile, projectPath]);

  const refreshWorkspaceScene = useCallback(async () => {
    if (!projectInfo || !projectPath) return;
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_status", { projectPath });
      setRuntimeStatus(status);
      await refreshScene(projectInfo, status);
    } catch {
      setRuntimeStatus(null);
      await refreshScene(projectInfo, null);
    }
  }, [projectInfo, projectPath, refreshScene]);

  const triggerBuild = useCallback(async (options?: { reason?: string }) => {
    if (!projectPath) return;
    const reason = options?.reason?.trim();
    setBuilding(true);
    setBuildLog(reason ? `${reason}\n\nBuilding...\n` : "Building...\n");
    setShowBuildLog(true);
    try {
      const result = await invoke<ShadowBuildResult>("shadow_trigger_build", { projectPath });
      const summary = `[${result.success ? "OK" : "FAILED"}] ${result.duration_ms}ms\n\n${result.output}`;
      setBuildLog(reason ? `${reason}\n\n${summary}` : summary);
      setAutoBuildStatus(result.success
        ? reason ?? `Last build finished in ${result.duration_ms}ms.`
        : `${reason ?? "Build failed"} - see build log.`);
    } catch (error) {
      const message = `[ERROR] ${String(error)}`;
      setBuildLog(reason ? `${reason}\n\n${message}` : message);
      setAutoBuildStatus(`${reason ?? "Build failed"} - ${String(error)}`);
    } finally {
      setBuilding(false);
      await load();
    }
  }, [load, projectPath]);

  const loadRuntime = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_load", {
        projectPath,
        loadEntryScene: true,
      });
      setRuntimeStatus(status);
      if (projectInfo) {
        await refreshScene(projectInfo, status);
      }
    } finally {
      setRuntimeBusy(false);
    }
  }, [projectInfo, projectPath, refreshScene]);

  const stopRuntime = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    setRuntimePlaying(false);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_stop", { projectPath });
      setRuntimeStatus(status);
      if (projectInfo) {
        await refreshScene(projectInfo, status);
      }
    } finally {
      setRuntimeBusy(false);
    }
  }, [projectInfo, projectPath, refreshScene]);

  const stepRuntime = useCallback(async (refreshAfter = true) => {
    if (!projectPath) return;
    const parsedDelta = Number.parseFloat(runtimeDelta);
    const deltaTime = Number.isFinite(parsedDelta) ? parsedDelta : 1 / 60;
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_step", {
        projectPath,
        deltaTime,
      });
      setRuntimeStatus(status);
      if (refreshAfter && projectInfo) {
        await refreshScene(projectInfo, status);
      }
    } catch {
      // backend status is surfaced on the next refresh
    }
  }, [projectInfo, projectPath, refreshScene, runtimeDelta]);

  const saveRuntimeScene = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_save_scene", { projectPath });
      setRuntimeStatus(status);
      await load();
    } finally {
      setRuntimeBusy(false);
    }
  }, [load, projectPath]);

  const runSceneMutation = useCallback(async (
    successMessage: string,
    action: () => Promise<ShadowScene>,
    nextSelectionId?: string | null | ((scene: ShadowScene) => string | null | undefined),
  ) => {
    setSceneBusy(true);
    setSceneStatus(null);
    try {
      const nextScene = await action();
      setSceneDrafts({});
      setSceneStatus(successMessage);
      setScene(nextScene);
      const resolvedSelection = typeof nextSelectionId === "function"
        ? nextSelectionId(nextScene)
        : nextSelectionId;
      if (resolvedSelection !== undefined) {
        setSelectedEntityId(resolvedSelection);
      }
      await refreshWorkspaceScene();
    } catch (error) {
      setSceneStatus(`Failed: ${String(error)}`);
    } finally {
      setSceneBusy(false);
    }
  }, [refreshWorkspaceScene]);

  const addEntityToScene = useCallback(async () => {
    if (!projectPath) return;
    const trimmedName = newEntityName.trim();
    await runSceneMutation(
      `Added ${trimmedName || "new entity"}.`,
      () => invoke<ShadowScene>("shadow_scene_add_entity", {
        projectPath,
        name: trimmedName || null,
      }),
      (nextScene) => nextScene.entities[nextScene.entities.length - 1]?.id ?? null,
    );
    setNewEntityName("");
  }, [newEntityName, projectPath, runSceneMutation]);

  const removeEntityFromScene = useCallback(async (entityId: string, entityName: string) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Removed ${entityName}.`,
      () => invoke<ShadowScene>("shadow_scene_remove_entity", {
        projectPath,
        entityId,
      }),
      null,
    );
  }, [projectPath, runSceneMutation]);

  const renameSceneEntity = useCallback(async (entityId: string, name: string) => {
    if (!projectPath || !name.trim()) return;
    await runSceneMutation(
      `Renamed to ${name.trim()}.`,
      () => invoke<ShadowScene>("shadow_scene_set_entity_name", {
        projectPath,
        entityId,
        name,
      }),
      entityId,
    );
  }, [projectPath, runSceneMutation]);

  const addComponentToEntity = useCallback(async (entityId: string, componentType: string) => {
    if (!projectPath || !componentType.trim()) return;
    await runSceneMutation(
      `Added ${componentType}.`,
      () => invoke<ShadowScene>("shadow_scene_add_component", {
        projectPath,
        entityId,
        componentType,
      }),
      entityId,
    );
    setNewComponentType("");
  }, [projectPath, runSceneMutation]);

  const removeComponentFromEntity = useCallback(async (entityId: string, componentType: string) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Removed ${componentType}.`,
      () => invoke<ShadowScene>("shadow_scene_remove_component", {
        projectPath,
        entityId,
        componentType,
      }),
      entityId,
    );
  }, [projectPath, runSceneMutation]);

  const commitSceneField = useCallback(async (
    entityId: string,
    componentType: string,
    fieldName: string,
    value: string,
  ) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Updated ${componentType}.${fieldName}.`,
      () => invoke<ShadowScene>("shadow_scene_set_component_field", {
        projectPath,
        entityId,
        componentType,
        fieldName,
        value,
      }),
      entityId,
    );
  }, [projectPath, runSceneMutation]);

  const toggleRuntimePlay = useCallback(async () => {
    if (runtimePlaying) {
      setRuntimePlaying(false);
      return;
    }
    if (!runtimeStatus?.is_live) {
      await loadRuntime();
    }
    setRuntimePlaying(true);
  }, [loadRuntime, runtimePlaying, runtimeStatus?.is_live]);

  useEffect(() => {
    if (!runtimePlaying || !runtimeStatus?.is_live) return;
    const timer = window.setInterval(() => {
      void stepRuntime(true);
    }, 250);
    return () => window.clearInterval(timer);
  }, [runtimePlaying, runtimeStatus?.is_live, stepRuntime]);

  useEffect(() => {
    if (!autoBuildEnabled) {
      setQueuedAutoBuildReason(null);
      return;
    }
    if (building || !queuedAutoBuildReason || autoBuildManagedExternally) return;
    const reason = queuedAutoBuildReason;
    setQueuedAutoBuildReason(null);
    void triggerBuild({ reason });
  }, [autoBuildEnabled, autoBuildManagedExternally, building, queuedAutoBuildReason, triggerBuild]);

  useEffect(() => {
    if (!visible || !projectPath) return;

    invoke("watch_workspace", { rootPath: projectPath }).catch(() => {});
    const unlistenPromise = listen<WorkspaceFsChangeEvent>("workspace-fs-changed", (event) => {
      const relevantRelativePaths = event.payload.paths
        .map((path) => normalizeFsPath(path))
        .map((path) => path.startsWith(projectPath)
          ? normalizeFsPath(path.slice(projectPath.length)).replace(/^\/+/, "")
          : path,
        )
        .filter((path) => path.length > 0)
        .filter((path) => !path.startsWith("build/") && !path.startsWith(".shadoweditor/"));

      if (relevantRelativePaths.length === 0) return;

      if (runtimeStatus?.is_live || relevantRelativePaths.some(isSceneRefreshPath)) {
        if (dataRefreshTimerRef.current) {
          window.clearTimeout(dataRefreshTimerRef.current);
        }
        dataRefreshTimerRef.current = window.setTimeout(() => {
          if (!building && !runtimeBusy && !sceneBusy) {
            void refreshWorkspaceScene();
          }
        }, 160);
      }

      if (!autoBuildEnabled || autoBuildManagedExternally) {
        if (autoBuildManagedExternally) {
          setAutoBuildStatus("Sidebar Game panel is driving hot reload for this project.");
        }
        return;
      }

      const buildTriggerPaths = relevantRelativePaths.filter(isAutoBuildTriggerPath);
      if (buildTriggerPaths.length === 0) return;

      if (autoBuildTimerRef.current) {
        window.clearTimeout(autoBuildTimerRef.current);
      }
      autoBuildTimerRef.current = window.setTimeout(() => {
        const preview = buildTriggerPaths.slice(0, 2).join(", ");
        const suffix = buildTriggerPaths.length > 2 ? ` +${buildTriggerPaths.length - 2} more` : "";
        const reason = `Live rebuild: ${preview}${suffix}`;
        setAutoBuildStatus(`${reason} - compiling.`);
        if (building) {
          setQueuedAutoBuildReason(reason);
          return;
        }
        void triggerBuild({ reason });
      }, 650);
    });

    return () => {
      if (autoBuildTimerRef.current) {
        window.clearTimeout(autoBuildTimerRef.current);
        autoBuildTimerRef.current = null;
      }
      if (dataRefreshTimerRef.current) {
        window.clearTimeout(dataRefreshTimerRef.current);
        dataRefreshTimerRef.current = null;
      }
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [autoBuildEnabled, autoBuildManagedExternally, building, projectPath, refreshWorkspaceScene, runtimeBusy, runtimeStatus?.is_live, sceneBusy, triggerBuild, visible]);

  useEffect(() => {
    if (!visible || !projectInfo || !runtimeStatus?.is_live) return;
    const timer = window.setInterval(() => {
      if (!building && !runtimeBusy && !sceneBusy) {
        void refreshWorkspaceScene();
      }
    }, 700);
    return () => window.clearInterval(timer);
  }, [building, projectInfo, refreshWorkspaceScene, runtimeBusy, runtimeStatus?.is_live, sceneBusy, visible]);

  if (!visible) {
    return null;
  }

  if (!projectPath) {
    return (
      <div style={S.empty}>
        <div style={S.emptyIcon}>🎮</div>
        <div style={S.emptyTitle}>Game Editor</div>
        <div style={S.emptyText}>Open a project folder to get started.</div>
      </div>
    );
  }

  if (loadError) {
    return (
      <div style={S.empty}>
        <div style={{ ...S.emptyTitle, color: "#f87171" }}>Failed to load project</div>
        <div style={S.emptyText}>{loadError}</div>
      </div>
    );
  }

  if (!projectInfo) {
    return (
      <div style={S.empty}>
        <div style={S.emptyIcon}>📁</div>
        <div style={S.emptyTitle}>No Shadow project found</div>
        <div style={S.emptyText}>
          Add a <code>.shadow_project.toml</code> file to your project to enable the game editor.
        </div>
      </div>
    );
  }

  const isRuntimeLive = runtimeStatus?.is_live ?? false;
  const entityCount = runtimeStatus?.entity_count ?? scene?.entities.length ?? 0;
  const sceneLabel = scene?.scene_name || projectInfo.entry_scene || "Not loaded";
  const selectedEntitySummary = selectedEntity
    ? `${selectedEntity.components.length} component${selectedEntity.components.length === 1 ? "" : "s"}`
    : "Select an entity in the full workspace";
  const selectedEntityTypes = selectedEntity?.components
    .slice(0, 3)
    .map((component) => component.component_type)
    .join(" · ") ?? "";

  if (isDockedViewport) {
    return (
      <div style={S.dockRoot}>
        <div style={S.dockToolbar}>
          <div style={S.dockToolbarLeft}>
            <div style={S.dockEyebrow}>Realtime Viewport</div>
            <div style={S.dockTitleRow}>
              <div style={S.dockTitle}>{projectInfo.name}</div>
              <span style={S.badge}>{projectInfo.runtime}</span>
              <span style={{ ...S.badge, ...(isRuntimeLive ? S.dockLiveBadge : S.dockStoppedBadge) }}>
                {isRuntimeLive ? "Live" : "Authoring"}
              </span>
              <span style={{ ...S.badge, ...(terrainDetails ? S.dockTerrainBadge : S.dockStoppedBadge) }}>
                {terrainDetails ? terrainDetails.entityName : hasGroundSurface ? "Atmosphere + Ground" : "Scene Preview"}
              </span>
            </div>
            <div style={S.dockSubtitle}>
              Watch terrain, entities, and hot reloads update while you edit code in ShadowIDE.
            </div>
          </div>
          <div style={S.dockToolbarRight}>
            <button style={{ ...S.btn, ...S.btnPrimary }} disabled={building} onClick={() => { void triggerBuild(); }}>
              {building ? "Building..." : "Build"}
            </button>
            <button
              style={{ ...S.btn, ...(runtimePlaying ? S.btnDanger : S.btnAccent), opacity: runtimeBusy || !projectInfo.game_library_exists ? 0.45 : 1 }}
              disabled={runtimeBusy || !projectInfo.game_library_exists}
              onClick={() => { void toggleRuntimePlay(); }}
            >
              {runtimePlaying ? "Stop" : "Play"}
            </button>
            <button
              style={{ ...S.btn, ...(autoBuildEnabled && !autoBuildManagedExternally ? S.btnAccent : {}) }}
              disabled={autoBuildManagedExternally}
              onClick={() => setAutoBuildEnabled((value) => !value)}
            >
              {autoBuildManagedExternally ? "Sidebar Reload" : autoBuildEnabled ? "Hot Reload On" : "Hot Reload Off"}
            </button>
            <button style={S.btn} onClick={() => { void refreshWorkspaceScene(); }}>
              Refresh
            </button>
            <button style={S.btn} onClick={() => onRequestFullView?.()}>
              Full View
            </button>
            <button style={S.btn} onClick={() => onCloseDock?.()}>
              Hide
            </button>
          </div>
        </div>

        {runtimeStatus?.last_error && (
          <div style={{ ...S.notice, ...S.dockNotice, color: "#f87171" }}>{runtimeStatus.last_error}</div>
        )}
        {sceneStatus && (
          <div style={{ ...S.notice, ...S.dockNotice, color: sceneStatus.startsWith("Failed") ? "#f87171" : "#7ed4a7" }}>
            {sceneStatus}
          </div>
        )}
        {autoBuildStatus && (
          <div style={{ ...S.notice, ...S.dockNotice, color: autoBuildStatus.toLowerCase().includes("failed") || autoBuildStatus.toLowerCase().includes("error") ? "#f87171" : "#8eb5c4" }}>
            {autoBuildStatus}
          </div>
        )}

        <div style={S.dockViewportStage}>
          <div style={S.dockViewportShell}>
            <div
              ref={viewportRef}
              tabIndex={0}
              style={{ ...S.viewportFrame, ...S.dockViewportFrame }}
              onFocus={() => setViewportFocused(true)}
              onBlur={() => setViewportFocused(false)}
              onMouseDown={handleViewportPointerDown}
              onWheel={handleViewportWheel}
            >
              <canvas ref={canvasRef} style={S.viewportCanvas} />
              <div style={S.viewportOverlayTop}>
                <span style={{ ...S.viewportChip, ...(isRuntimeLive ? S.viewportChipLive : S.viewportChipMuted) }}>
                  {isRuntimeLive ? "LIVE RUNTIME" : sceneSource === "entry" ? "AUTHORED SCENE" : "NO SCENE"}
                </span>
                <span style={{ ...S.viewportChip, ...(terrainDetails ? S.viewportChipTerrain : S.viewportChipMuted) }}>
                  {terrainDetails ? `${terrainDetails.cols} x ${terrainDetails.rows}` : hasGroundSurface ? "Ground + Sky" : "Sky Preview"}
                </span>
                <span style={{ ...S.viewportChip, ...(autoBuildEnabled || autoBuildManagedExternally ? S.viewportChipLive : S.viewportChipMuted) }}>
                  {autoBuildManagedExternally ? "Sidebar Hot Reload" : autoBuildEnabled ? "Live Sync Ready" : "Manual Rebuild"}
                </span>
                <span style={{ ...S.viewportChip, ...S.viewportChipMode }}>
                  {cameraMode === "orbit" ? "ORBIT" : cameraMode === "third_person" ? "THIRD PERSON" : "FIRST PERSON"}
                </span>
              </div>
              <div style={S.viewportModeBar} onMouseDown={(event) => event.stopPropagation()}>
                <button style={{ ...S.viewportModeButton, ...(cameraMode === "orbit" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("orbit")}>Orbit</button>
                <button style={{ ...S.viewportModeButton, ...(cameraMode === "third_person" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("third_person")}>3rd Person</button>
                <button style={{ ...S.viewportModeButton, ...(cameraMode === "first_person" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("first_person")}>1st Person</button>
                <button style={S.viewportModeButton} onClick={() => resetViewportCamera()}>Reset</button>
              </div>
              <div style={S.dockViewportBottomOverlay}>
                <div style={S.dockOverlayCard}>
                  <div style={S.dockOverlayLabel}>Scene</div>
                  <div style={S.dockOverlayValue}>{sceneLabel}</div>
                </div>
                <div style={S.dockOverlayCard}>
                  <div style={S.dockOverlayLabel}>Selected</div>
                  <div style={S.dockOverlayValue}>{selectedEntity?.name || "No entity selected"}</div>
                </div>
                <div style={S.dockOverlayCard}>
                  <div style={S.dockOverlayLabel}>Runtime</div>
                  <div style={S.dockOverlayValue}>{runtimeStatus?.status_line || "Ready to author"}</div>
                </div>
              </div>
              {!terrainDetails && (
                <div style={S.viewportEmptyCard}>
                  <div style={S.viewportEmptyTitle}>{scene?.entities.length ? "Scene loaded" : "Viewport ready"}</div>
                  <div style={S.viewportEmptyText}>
                    {scene?.entities.length
                      ? hasGroundSurface
                        ? "The viewport is rendering the scene from entity transforms, the editor atmosphere, and the inferred ground surface while you code."
                        : "A dedicated terrain component is not present yet, but the live atmosphere, entity transforms, and hot-reloaded scene updates still render here."
                      : "Build the runtime and press Play to watch changes land here while you keep coding."}
                  </div>
                </div>
              )}
            </div>

            <div style={S.dockFooter}>
              <div style={S.dockInfoCard}>
                <div style={S.dockCardLabel}>Selection</div>
                <div style={S.dockCardValue}>{selectedEntity?.name || "No entity selected"}</div>
                <div style={S.dockCardMeta}>
                  {selectedEntity ? `${selectedEntitySummary}${selectedEntityTypes ? ` · ${selectedEntityTypes}` : ""}` : "Open Full View to inspect and edit components."}
                </div>
              </div>
              <div style={S.dockInfoGrid}>
                <div style={S.dockMiniStat}>
                  <div style={S.dockMiniStatLabel}>Entities</div>
                  <div style={S.dockMiniStatValue}>{String(entityCount)}</div>
                </div>
                <div style={S.dockMiniStat}>
                  <div style={S.dockMiniStatLabel}>Components</div>
                  <div style={S.dockMiniStatValue}>{String(runtimeStatus?.component_count ?? 0)}</div>
                </div>
                <div style={S.dockMiniStat}>
                  <div style={S.dockMiniStatLabel}>Frame</div>
                  <div style={S.dockMiniStatValue}>{String(runtimeStatus?.frame_index ?? 0)}</div>
                </div>
                <div style={S.dockMiniStat}>
                  <div style={S.dockMiniStatLabel}>Source</div>
                  <div style={S.dockMiniStatValue}>{primarySource?.path.split("/").pop() || "game.cpp"}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div style={S.root}>
      {/* Header bar */}
      <div style={S.header}>
        <div style={S.headerLeft}>
          <div style={S.projectName}>{projectInfo.name}</div>
          <span style={S.badge}>{projectInfo.runtime}</span>
          <span style={{ ...S.badge, borderColor: isRuntimeLive ? "#7ed4a755" : "#ffffff15", color: isRuntimeLive ? "#7ed4a7" : "#8eb5c4" }}>
            {isRuntimeLive ? "Live" : "Stopped"}
          </span>
          {sceneSource !== "none" && (
            <span style={{ ...S.badge, borderColor: "#7eb8d433", color: "#7eb8d4" }}>
              {scene?.scene_name || "Scene"}
            </span>
          )}
        </div>
        <div style={S.headerRight}>
          <span style={S.headerStat}>{entityCount} entities</span>
        </div>
      </div>

      {/* Primary toolbar */}
      <div style={S.toolbar}>
        <button style={{ ...S.btn, ...S.btnPrimary }} disabled={building} onClick={() => { void triggerBuild(); }}>
          {building ? "Building..." : "Build"}
        </button>
        <button
          style={{ ...S.btn, ...(runtimePlaying ? S.btnDanger : S.btnPrimary), opacity: runtimeBusy || !projectInfo.game_library_exists ? 0.45 : 1 }}
          disabled={runtimeBusy || !projectInfo.game_library_exists}
          onClick={() => { void toggleRuntimePlay(); }}
        >
          {runtimePlaying ? "Stop" : "Play"}
        </button>
        <button
          style={{ ...S.btn, ...S.btnAccent }}
          onClick={() => onActivatePanel?.("planengine")}
        >
          PlanEngine
        </button>
        <button style={S.btn} onClick={() => onActivatePanel?.("ai")}>
          AI Chat
        </button>
        {primarySource && (
          <button style={S.btn} onClick={() => openProjectFile(primarySource.path)}>
            Open Source
          </button>
        )}

        <div style={S.toolbarSpacer} />

        <button
          style={{ ...S.btn, fontSize: 11, color: "#8eb5c4" }}
          onClick={() => setShowAdvanced(!showAdvanced)}
        >
          {showAdvanced ? "Less" : "More"}
        </button>
      </div>

      {/* Advanced toolbar (collapsed by default) */}
      {showAdvanced && (
        <div style={S.advancedBar}>
          <button style={{ ...S.btn, opacity: runtimeBusy || !projectInfo.game_library_exists ? 0.45 : 1 }} disabled={runtimeBusy || !projectInfo.game_library_exists} onClick={() => { void loadRuntime(); }}>
            {isRuntimeLive ? "Reload" : "Load Runtime"}
          </button>
          <button style={{ ...S.btn, opacity: !isRuntimeLive ? 0.45 : 1 }} disabled={!isRuntimeLive} onClick={() => { void stepRuntime(true); }}>
            Step
          </button>
          <button style={{ ...S.btn, opacity: !isRuntimeLive ? 0.45 : 1 }} disabled={!isRuntimeLive} onClick={() => { void saveRuntimeScene(); }}>
            Save Scene
          </button>
          <button style={{ ...S.btn, opacity: !isRuntimeLive ? 0.45 : 1 }} disabled={!isRuntimeLive} onClick={() => { void stopRuntime(); }}>
            Stop Runtime
          </button>
          <button style={S.btn} onClick={() => openProjectFile(projectInfo.entry_scene_path)}>
            Open Scene File
          </button>
          {primarySource && (
            <button style={S.btn} onClick={() => openProjectFile(primarySource.path)}>
              Open Source
            </button>
          )}
          {primaryHeader && (
            <button style={S.btn} onClick={() => openProjectFile(primaryHeader.path)}>
              Open Header
            </button>
          )}
          <button style={{ ...S.btn, ...(autoBuildEnabled && !autoBuildManagedExternally ? S.btnAccent : {}) }} onClick={() => setAutoBuildEnabled((value) => !value)}>
            {autoBuildManagedExternally ? "Sidebar Hot Reload" : autoBuildEnabled ? "Hot Reload On" : "Hot Reload Off"}
          </button>
          <button style={S.btn} onClick={() => { void refreshWorkspaceScene(); }}>
            Refresh View
          </button>
          <div style={S.toolbarSpacer} />
          <label style={S.deltaLabel}>
            dt
            <input
              style={S.deltaInput}
              value={runtimeDelta}
              onChange={(event) => setRuntimeDelta(event.target.value)}
              spellCheck={false}
            />
          </label>
          <span style={S.frameMeta}>frame {runtimeStatus?.frame_index ?? 0}</span>
        </div>
      )}

      {/* Status notices */}
      {runtimeStatus?.last_error && (
        <div style={{ ...S.notice, color: "#f87171" }}>{runtimeStatus.last_error}</div>
      )}
      {sceneStatus && (
        <div style={{ ...S.notice, color: sceneStatus.startsWith("Failed") ? "#f87171" : "#7ed4a7" }}>
          {sceneStatus}
        </div>
      )}
      {autoBuildStatus && (
        <div style={{ ...S.notice, color: autoBuildStatus.toLowerCase().includes("failed") || autoBuildStatus.toLowerCase().includes("error") ? "#f87171" : "#8eb5c4" }}>
          {autoBuildStatus}
        </div>
      )}

      {/* Main content grid */}
      <div style={S.grid}>
        {/* Hierarchy panel */}
        <section style={S.panel}>
          <div style={S.panelHeader}>
            <span>Hierarchy</span>
            <span style={S.panelCount}>{scene?.entities.length ?? 0}</span>
          </div>
          <div style={S.panelToolbar}>
            <input
              style={{ ...S.input, flex: 1 }}
              value={newEntityName}
              onChange={(event) => setNewEntityName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void addEntityToScene();
                }
              }}
              placeholder="New entity name..."
              spellCheck={false}
            />
            <button
              style={{ ...S.btn, opacity: sceneBusy ? 0.45 : 1 }}
              disabled={sceneBusy}
              onClick={() => { void addEntityToScene(); }}
            >
              Add
            </button>
          </div>
          {!scene?.entities.length ? (
            <div style={S.placeholder}>No entities yet. Add one above to get started.</div>
          ) : (
            <div style={S.entityList}>
              {scene.entities.map((entity) => {
                const selected = entity.id === selectedEntity?.id;
                return (
                  <button
                    key={entity.id}
                    style={{ ...S.entityRow, ...(selected ? S.entityRowSelected : {}) }}
                    onClick={() => setSelectedEntityId(entity.id)}
                  >
                    <span style={S.entityName}>{entity.name || entity.id}</span>
                    <span style={S.entityMeta}>{entity.components.length}</span>
                  </button>
                );
              })}
            </div>
          )}
        </section>

        {/* Center panel - live 3D viewport */}
        <section style={S.panel}>
          <div style={S.panelHeader}>
            <span>Live View</span>
            <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <button
                style={{ ...S.viewportModeButton, ...(viewportRenderer === "3d" ? S.viewportModeButtonActive : {}), padding: "2px 8px", fontSize: 10 }}
                onClick={() => setViewportRenderer("3d")}
              >
                3D
              </button>
              <button
                style={{ ...S.viewportModeButton, ...(viewportRenderer === "scene" ? S.viewportModeButtonActive : {}), padding: "2px 8px", fontSize: 10 }}
                onClick={() => setViewportRenderer("scene")}
              >
                Scene
              </button>
              <span style={S.panelCount}>
                {isRuntimeLive ? "LIVE" : terrainDetails ? `${terrainDetails.cols}x${terrainDetails.rows}` : sceneSource === "none" ? "No scene" : "Scene view"}
              </span>
            </div>
          </div>
          <div style={S.centerPanel}>
            {viewportRenderer === "3d" ? (
              <Viewport3D
                entities={viewportEntities}
                selectedEntityId={selectedEntityId}
                onSelectEntity={setSelectedEntityId}
                style={{ width: "100%", height: "100%", minHeight: 300 }}
              />
            ) : (
              <>
                <div style={S.viewportShell}>
                  <div
                    ref={viewportRef}
                    tabIndex={0}
                    style={S.viewportFrame}
                    onFocus={() => setViewportFocused(true)}
                    onBlur={() => setViewportFocused(false)}
                    onMouseDown={handleViewportPointerDown}
                    onWheel={handleViewportWheel}
                  >
                    <canvas ref={canvasRef} style={S.viewportCanvas} />
                    <div style={S.viewportOverlayTop}>
                      <span style={{ ...S.viewportChip, ...(isRuntimeLive ? S.viewportChipLive : S.viewportChipMuted) }}>
                        {isRuntimeLive ? "LIVE" : sceneSource === "entry" ? "SCENE" : "EMPTY"}
                      </span>
                      <span style={{ ...S.viewportChip, ...S.viewportChipMode }}>
                        {cameraMode === "orbit" ? "ORBIT" : cameraMode === "third_person" ? "3RD PERSON" : "1ST PERSON"}
                      </span>
                    </div>
                    <div style={S.viewportModeBar} onMouseDown={(event) => event.stopPropagation()}>
                      <button style={{ ...S.viewportModeButton, ...(cameraMode === "orbit" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("orbit")}>Orbit</button>
                      <button style={{ ...S.viewportModeButton, ...(cameraMode === "third_person" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("third_person")}>3rd Person</button>
                      <button style={{ ...S.viewportModeButton, ...(cameraMode === "first_person" ? S.viewportModeButtonActive : {}) }} onClick={() => setCameraMode("first_person")}>1st Person</button>
                      <button style={S.viewportModeButton} onClick={() => resetViewportCamera()}>Reset</button>
                    </div>
                  </div>
                  <div style={S.viewportStats}>
                    <div style={S.viewportStatCard}>
                      <div style={S.viewportStatLabel}>Scene</div>
                      <div style={S.viewportStatValue}>{scene?.scene_name || projectInfo.entry_scene || "—"}</div>
                    </div>
                    <div style={S.viewportStatCard}>
                      <div style={S.viewportStatLabel}>Entities</div>
                      <div style={S.viewportStatValue}>{runtimeStatus?.entity_count ?? scene?.entities.length ?? 0}</div>
                    </div>
                    <div style={S.viewportStatCard}>
                      <div style={S.viewportStatLabel}>Frame</div>
                      <div style={S.viewportStatValue}>{String(runtimeStatus?.frame_index ?? 0)}</div>
                    </div>
                  </div>
                </div>
              </>
            )}
          </div>
        </section>

        {/* Inspector panel */}
        <section style={S.panel}>
          <div style={S.panelHeader}>Inspector</div>
          {!selectedEntity ? (
            <div style={S.placeholder}>Select an entity to see its components.</div>
          ) : (
            <div style={S.inspector}>
              <div style={S.inspectorHeader}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <input
                    style={S.entityNameInput}
                    value={sceneDrafts[sceneEntityDraftKey(selectedEntity.id)] ?? selectedEntity.name}
                    onChange={(event) => setSceneDrafts((current) => ({
                      ...current,
                      [sceneEntityDraftKey(selectedEntity.id)]: event.target.value,
                    }))}
                    onBlur={(event) => {
                      const nextValue = event.target.value.trim();
                      if (nextValue && nextValue !== selectedEntity.name) {
                        void renameSceneEntity(selectedEntity.id, nextValue);
                      }
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.currentTarget.blur();
                      }
                    }}
                    spellCheck={false}
                  />
                  <div style={S.inspectorId}>{selectedEntity.id}</div>
                </div>
                <button
                  style={{ ...S.btnRemove, opacity: sceneBusy ? 0.45 : 1 }}
                  disabled={sceneBusy}
                  onClick={() => { void removeEntityFromScene(selectedEntity.id, selectedEntity.name || selectedEntity.id); }}
                >
                  Remove
                </button>
              </div>

              <div style={S.panelToolbar}>
                <select
                  style={{ ...S.select, flex: 1 }}
                  value={newComponentType || addableComponentTypes[0] || ""}
                  onChange={(event) => setNewComponentType(event.target.value)}
                  disabled={sceneBusy || addableComponentTypes.length === 0}
                >
                  {addableComponentTypes.length === 0 ? (
                    <option value="">No more components</option>
                  ) : (
                    addableComponentTypes.map((componentType) => (
                      <option key={componentType} value={componentType}>
                        {componentType}
                      </option>
                    ))
                  )}
                </select>
                <button
                  style={{ ...S.btn, opacity: (sceneBusy || addableComponentTypes.length === 0) ? 0.45 : 1 }}
                  disabled={sceneBusy || addableComponentTypes.length === 0}
                  onClick={() => {
                    const componentType = newComponentType || addableComponentTypes[0];
                    if (componentType) {
                      void addComponentToEntity(selectedEntity.id, componentType);
                    }
                  }}
                >
                  Add
                </button>
              </div>

              {selectedEntity.components.length === 0 ? (
                <div style={S.placeholder}>No components yet.</div>
              ) : (
                selectedEntity.components.map((component) => (
                  <div key={`${selectedEntity.id}-${component.component_type}`} style={S.componentCard}>
                    <div style={S.componentHeader}>
                      <div style={S.componentName}>{component.component_type}</div>
                      <button
                        style={{ ...S.btnRemoveSmall, opacity: sceneBusy ? 0.45 : 1 }}
                        disabled={sceneBusy}
                        onClick={() => { void removeComponentFromEntity(selectedEntity.id, component.component_type); }}
                      >
                        x
                      </button>
                    </div>
                    {component.fields.length === 0 ? (
                      <div style={S.componentEmpty}>no fields</div>
                    ) : (
                      component.fields.map(([fieldName, fieldValue]) => (
                        <div key={`${component.component_type}-${fieldName}`} style={S.fieldRow}>
                          <span style={S.fieldName}>{fieldName}</span>
                          <input
                            style={S.fieldInput}
                            value={sceneDrafts[sceneFieldDraftKey(selectedEntity.id, component.component_type, fieldName)] ?? fieldValue}
                            onChange={(event) => setSceneDrafts((current) => ({
                              ...current,
                              [sceneFieldDraftKey(selectedEntity.id, component.component_type, fieldName)]: event.target.value,
                            }))}
                            onBlur={(event) => {
                              const nextValue = event.target.value;
                              if (nextValue !== fieldValue) {
                                void commitSceneField(selectedEntity.id, component.component_type, fieldName, nextValue);
                              }
                            }}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") {
                                event.currentTarget.blur();
                              }
                            }}
                            spellCheck={false}
                          />
                        </div>
                      ))
                    )}
                  </div>
                ))
              )}
            </div>
          )}
        </section>
      </div>

      {/* Collapsible build log */}
      <div style={S.logToggle}>
        <button style={S.logToggleBtn} onClick={() => setShowBuildLog(!showBuildLog)}>
          {showBuildLog ? "Hide" : "Show"} Build Log
        </button>
      </div>
      {showBuildLog && (
        <section style={S.logPanel}>
          <pre style={S.logOutput}>{buildLog || "No build output yet."}</pre>
        </section>
      )}
    </div>
  );
}

/* ─── Styles ─── */

const S: Record<string, React.CSSProperties> = {
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    background: "#0b1217",
    color: "#ecf1f4",
    overflow: "hidden",
  },
  empty: {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    height: "100%",
    padding: 24,
    background: "#0b1217",
    color: "#ecf1f4",
    textAlign: "center",
    gap: 8,
  },
  emptyIcon: { fontSize: 32 },
  emptyTitle: { fontSize: 16, fontWeight: 700, color: "#e9aa5f" },
  emptyText: { fontSize: 12, color: "#8eb5c4", lineHeight: 1.6, maxWidth: 400 },

  /* Header */
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    padding: "10px 16px",
    borderBottom: "1px solid #1c2a31",
    gap: 12,
  },
  headerLeft: { display: "flex", alignItems: "center", gap: 8, minWidth: 0 },
  headerRight: { display: "flex", alignItems: "center", gap: 8, flexShrink: 0 },
  projectName: { fontSize: 15, fontWeight: 700, color: "#f4f7f8" },
  badge: {
    fontSize: 10,
    color: "#e9aa5f",
    background: "#e9aa5f12",
    border: "1px solid #e9aa5f33",
    borderRadius: 999,
    padding: "2px 8px",
    whiteSpace: "nowrap",
  },
  headerStat: { fontSize: 11, color: "#8eb5c4" },

  /* Toolbar */
  toolbar: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    padding: "8px 16px",
  },
  advancedBar: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 6,
    padding: "0 16px 8px",
  },
  toolbarSpacer: { flex: 1 },
  btn: {
    padding: "5px 10px",
    borderRadius: 6,
    border: "1px solid #24343d",
    background: "#101920",
    color: "#d8e2e7",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    whiteSpace: "nowrap",
  },
  btnPrimary: {
    borderColor: "#e9aa5f44",
    color: "#e9aa5f",
  },
  btnAccent: {
    borderColor: "#7eb8d444",
    color: "#7eb8d4",
  },
  btnDanger: {
    borderColor: "#f8717144",
    color: "#f87171",
  },
  deltaLabel: {
    display: "flex",
    alignItems: "center",
    gap: 4,
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 1,
  },
  deltaInput: {
    width: 60,
    background: "#081015",
    border: "1px solid #24343d",
    borderRadius: 6,
    color: "#ecf1f4",
    fontSize: 11,
    padding: "4px 6px",
    outline: "none",
  },
  frameMeta: { fontSize: 10, color: "#6f8792" },

  /* Notices */
  notice: {
    margin: "0 16px 4px",
    padding: "6px 10px",
    borderRadius: 8,
    border: "1px solid #24343d",
    background: "#101920",
    fontSize: 11,
  },
  dockNotice: {
    margin: "0 16px",
  },

  /* Docked viewport mode */
  dockRoot: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    minHeight: 0,
    background:
      "radial-gradient(circle at top right, rgba(126, 184, 212, 0.15), transparent 30%), linear-gradient(180deg, #081117 0%, #070d13 100%)",
    color: "#ecf1f4",
    overflow: "hidden",
  },
  dockToolbar: {
    display: "flex",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 12,
    padding: "14px 16px 12px",
    borderBottom: "1px solid #18232b",
  },
  dockToolbarLeft: {
    display: "flex",
    flexDirection: "column",
    gap: 6,
    minWidth: 0,
    flex: 1,
  },
  dockEyebrow: {
    fontSize: 10,
    fontWeight: 800,
    letterSpacing: 1.2,
    textTransform: "uppercase",
    color: "#8eb5c4",
  },
  dockTitleRow: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
  },
  dockTitle: {
    fontSize: 18,
    fontWeight: 800,
    color: "#f4f7f8",
  },
  dockSubtitle: {
    fontSize: 12,
    color: "#89a6b4",
    maxWidth: 520,
    lineHeight: 1.5,
  },
  dockToolbarRight: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    justifyContent: "flex-end",
    gap: 6,
    flexShrink: 0,
  },
  dockLiveBadge: {
    borderColor: "#7ed4a744",
    color: "#7ed4a7",
    background: "rgba(126, 212, 167, 0.08)",
  },
  dockStoppedBadge: {
    borderColor: "#24343d",
    color: "#8eb5c4",
    background: "rgba(8, 16, 21, 0.75)",
  },
  dockTerrainBadge: {
    borderColor: "#e9aa5f44",
    color: "#e9aa5f",
    background: "rgba(233, 170, 95, 0.08)",
  },
  dockViewportStage: {
    flex: 1,
    minHeight: 0,
    padding: 16,
    display: "flex",
    flexDirection: "column",
  },
  dockViewportShell: {
    display: "flex",
    flexDirection: "column",
    gap: 12,
    minHeight: 0,
    flex: 1,
  },
  dockViewportFrame: {
    minHeight: 0,
    height: "100%",
    flex: 1,
    borderColor: "#1b2a33",
    boxShadow: "inset 0 1px 0 rgba(255,255,255,0.03), 0 20px 50px rgba(0,0,0,0.25)",
  },
  dockViewportBottomOverlay: {
    position: "absolute",
    left: 16,
    right: 16,
    bottom: 16,
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: 8,
    pointerEvents: "none",
  },
  dockOverlayCard: {
    minWidth: 0,
    padding: "10px 12px",
    borderRadius: 10,
    border: "1px solid rgba(36, 52, 61, 0.9)",
    background: "rgba(8, 16, 21, 0.8)",
    backdropFilter: "blur(10px)",
  },
  dockOverlayLabel: {
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 0.55,
    marginBottom: 4,
  },
  dockOverlayValue: {
    fontSize: 12,
    fontWeight: 700,
    color: "#f4f7f8",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  dockFooter: {
    display: "grid",
    gridTemplateColumns: "minmax(220px, 1.1fr) minmax(240px, 1fr)",
    gap: 10,
  },
  dockInfoCard: {
    padding: "12px 14px",
    borderRadius: 10,
    border: "1px solid #1c2a31",
    background: "rgba(8, 16, 21, 0.88)",
    minWidth: 0,
  },
  dockCardLabel: {
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 0.6,
    marginBottom: 6,
  },
  dockCardValue: {
    fontSize: 14,
    fontWeight: 800,
    color: "#f4f7f8",
    marginBottom: 6,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  dockCardMeta: {
    fontSize: 11,
    lineHeight: 1.5,
    color: "#8eb5c4",
  },
  dockInfoGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(4, minmax(0, 1fr))",
    gap: 8,
  },
  dockMiniStat: {
    padding: "10px 12px",
    borderRadius: 10,
    border: "1px solid #1c2a31",
    background: "rgba(8, 16, 21, 0.88)",
    minWidth: 0,
  },
  dockMiniStatLabel: {
    fontSize: 10,
    color: "#6f8792",
    textTransform: "uppercase",
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  dockMiniStatValue: {
    fontSize: 13,
    fontWeight: 800,
    color: "#ecf1f4",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },

  /* Grid */
  grid: {
    display: "grid",
    gridTemplateColumns: "minmax(220px, 0.8fr) minmax(300px, 1.2fr) minmax(260px, 1fr)",
    gap: 8,
    padding: "8px 16px",
    minHeight: 0,
    flex: 1,
  },
  panel: {
    minWidth: 0,
    minHeight: 0,
    display: "flex",
    flexDirection: "column",
    borderRadius: 10,
    border: "1px solid #1c2a31",
    background: "#0c1419",
    overflow: "hidden",
  },
  panelHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    padding: "8px 12px",
    borderBottom: "1px solid #17252c",
    fontSize: 11,
    fontWeight: 700,
    letterSpacing: 0.5,
    textTransform: "uppercase",
    color: "#8eb5c4",
  },
  panelCount: {
    fontSize: 10,
    color: "#6f8792",
    fontWeight: 400,
  },
  panelToolbar: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    padding: "8px 10px 4px",
  },
  placeholder: {
    padding: 16,
    fontSize: 12,
    color: "#6f8792",
    textAlign: "center",
  },

  /* Entity list */
  entityList: {
    display: "flex",
    flexDirection: "column",
    gap: 4,
    padding: "4px 8px 8px",
    overflowY: "auto",
  },
  entityRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 6,
    padding: "6px 10px",
    borderRadius: 6,
    border: "1px solid transparent",
    background: "#0f181d",
    color: "#ecf1f4",
    cursor: "pointer",
    textAlign: "left",
  },
  entityRowSelected: {
    borderColor: "#e9aa5f44",
    background: "rgba(233,170,95,0.08)",
  },
  entityName: {
    fontSize: 12,
    fontWeight: 600,
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  entityMeta: { fontSize: 10, color: "#6f8792", flexShrink: 0 },

  /* Center panel - getting started / runtime info */
  centerPanel: {
    flex: 1,
    display: "flex",
    flexDirection: "column",
    alignItems: "stretch",
    justifyContent: "flex-start",
    gap: 16,
    padding: 16,
    overflowY: "auto",
  },
  viewportShell: {
    display: "flex",
    flexDirection: "column",
    gap: 12,
  },
  viewportFrame: {
    position: "relative",
    minHeight: 320,
    borderRadius: 10,
    overflow: "hidden",
    border: "1px solid #1c2a31",
    background: "#0b1217",
    boxShadow: "inset 0 1px 0 rgba(255,255,255,0.03)",
    outline: "none",
  },
  viewportCanvas: {
    display: "block",
    width: "100%",
    height: "100%",
    minHeight: 320,
  },
  viewportOverlayTop: {
    position: "absolute",
    top: 12,
    left: 12,
    right: 12,
    display: "flex",
    flexWrap: "wrap",
    gap: 8,
    pointerEvents: "none",
  },
  viewportChip: {
    padding: "4px 9px",
    borderRadius: 999,
    border: "1px solid #24343d",
    background: "rgba(8,16,21,0.82)",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: 0.35,
    whiteSpace: "nowrap",
  },
  viewportChipLive: {
    borderColor: "#7ed4a744",
    color: "#7ed4a7",
  },
  viewportChipMuted: {
    borderColor: "#24343d",
    color: "#8eb5c4",
  },
  viewportChipTerrain: {
    borderColor: "#e9aa5f44",
    color: "#e9aa5f",
  },
  viewportChipMode: {
    borderColor: "#7eb8d444",
    color: "#d4e7f2",
  },
  viewportModeBar: {
    position: "absolute",
    top: 48,
    left: 12,
    display: "flex",
    alignItems: "center",
    gap: 6,
    zIndex: 2,
  },
  viewportModeButton: {
    padding: "5px 9px",
    borderRadius: 999,
    border: "1px solid rgba(36, 52, 61, 0.92)",
    background: "rgba(8, 16, 21, 0.84)",
    color: "#c9d7de",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: 0.3,
    cursor: "pointer",
  },
  viewportModeButtonActive: {
    borderColor: "#e9aa5f55",
    color: "#ffd39a",
    background: "rgba(233, 170, 95, 0.16)",
  },
  viewportEmptyCard: {
    position: "absolute",
    left: 16,
    bottom: 16,
    maxWidth: 320,
    padding: "12px 14px",
    borderRadius: 10,
    border: "1px solid #24343d",
    background: "rgba(11, 18, 23, 0.88)",
    backdropFilter: "blur(8px)",
  },
  viewportEmptyTitle: {
    fontSize: 13,
    fontWeight: 700,
    color: "#f4f7f8",
    marginBottom: 6,
  },
  viewportEmptyText: {
    fontSize: 11,
    lineHeight: 1.55,
    color: "#9eb5c1",
  },
  viewportStats: {
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: 8,
  },
  viewportStatCard: {
    padding: "10px 12px",
    borderRadius: 8,
    border: "1px solid #1c2a31",
    background: "#0a1117",
    minWidth: 0,
  },
  viewportStatLabel: {
    fontSize: 10,
    color: "#6f8792",
    textTransform: "uppercase",
    letterSpacing: 0.45,
    marginBottom: 6,
  },
  viewportStatValue: {
    fontSize: 12,
    fontWeight: 700,
    color: "#ecf1f4",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  viewportHint: {
    gridColumn: "1 / -1",
    fontSize: 11,
    lineHeight: 1.55,
    color: "#8eb5c4",
    padding: "2px 2px 0",
  },
  gettingStarted: {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: 12,
    maxWidth: 360,
    textAlign: "center",
  },
  gsIcon: { fontSize: 36 },
  gsTitle: { fontSize: 16, fontWeight: 700, color: "#f4f7f8" },
  gsText: { fontSize: 12, color: "#8eb5c4", lineHeight: 1.6 },
  gsInfo: {
    width: "100%",
    display: "flex",
    flexDirection: "column",
    gap: 6,
    padding: 12,
    borderRadius: 8,
    border: "1px solid #1c2a31",
    background: "#0a1117",
    textAlign: "left",
  },
  gsInfoRow: {
    display: "flex",
    justifyContent: "space-between",
    fontSize: 11,
    color: "#c9d7de",
  },
  gsKey: { color: "#8eb5c4", fontWeight: 600 },

  runtimeInfo: {
    width: "100%",
    display: "flex",
    flexDirection: "column",
    gap: 16,
  },
  riHeader: {
    display: "flex",
    alignItems: "center",
    gap: 10,
  },
  riBadge: {
    fontSize: 10,
    fontWeight: 800,
    color: "#7ed4a7",
    background: "#7ed4a718",
    border: "1px solid #7ed4a744",
    borderRadius: 4,
    padding: "2px 8px",
    letterSpacing: 1,
  },
  riScene: { fontSize: 12, color: "#c9d7de", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  riStats: {
    display: "grid",
    gridTemplateColumns: "repeat(3, 1fr)",
    gap: 8,
  },
  riStat: {
    padding: 10,
    borderRadius: 8,
    border: "1px solid #1c2a31",
    background: "#0a1117",
    textAlign: "center",
  },
  riStatValue: { fontSize: 20, fontWeight: 800, color: "#f4f7f8" },
  riStatLabel: { fontSize: 10, color: "#8eb5c4", textTransform: "uppercase", marginTop: 4 },
  riStatus: { fontSize: 11, color: "#8eb5c4", textAlign: "center" },

  /* Inspector */
  inspector: {
    display: "flex",
    flexDirection: "column",
    gap: 10,
    padding: 10,
    overflowY: "auto",
  },
  inspectorHeader: {
    display: "flex",
    alignItems: "flex-start",
    gap: 8,
  },
  inspectorId: { fontSize: 10, color: "#6f8792", fontFamily: "monospace" },
  entityNameInput: {
    width: "100%",
    background: "#081015",
    border: "1px solid #24343d",
    borderRadius: 6,
    color: "#f3f6f8",
    fontSize: 14,
    fontWeight: 700,
    padding: "6px 8px",
    outline: "none",
    boxSizing: "border-box",
    marginBottom: 2,
  },
  componentCard: {
    padding: "8px 10px",
    borderRadius: 8,
    border: "1px solid #1c2b33",
    background: "#101920",
  },
  componentHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    marginBottom: 8,
  },
  componentName: { fontSize: 12, fontWeight: 700, color: "#e9aa5f" },
  componentEmpty: { fontSize: 10, color: "#6f8792" },
  fieldRow: {
    display: "grid",
    gridTemplateColumns: "100px minmax(0, 1fr)",
    gap: 6,
    padding: "3px 0",
    borderBottom: "1px solid rgba(142,181,196,0.06)",
  },
  fieldName: { fontSize: 10, color: "#8eb5c4", fontFamily: "monospace" },
  fieldInput: {
    width: "100%",
    background: "#081015",
    border: "1px solid #24343d",
    borderRadius: 6,
    color: "#ecf1f4",
    fontSize: 11,
    fontFamily: "monospace",
    padding: "5px 7px",
    outline: "none",
    boxSizing: "border-box",
  },
  input: {
    minWidth: 0,
    background: "#081015",
    border: "1px solid #24343d",
    borderRadius: 6,
    color: "#ecf1f4",
    fontSize: 12,
    padding: "5px 8px",
    outline: "none",
    boxSizing: "border-box",
  },
  select: {
    minWidth: 0,
    background: "#081015",
    border: "1px solid #24343d",
    borderRadius: 6,
    color: "#ecf1f4",
    fontSize: 12,
    padding: "5px 8px",
    outline: "none",
    boxSizing: "border-box",
  },
  btnRemove: {
    padding: "5px 8px",
    borderRadius: 6,
    border: "1px solid #5a2020",
    background: "transparent",
    color: "#f87171",
    fontSize: 11,
    fontWeight: 600,
    cursor: "pointer",
  },
  btnRemoveSmall: {
    padding: "2px 6px",
    borderRadius: 4,
    border: "1px solid #5a2020",
    background: "transparent",
    color: "#f87171",
    fontSize: 10,
    cursor: "pointer",
  },

  /* Build log */
  logToggle: {
    padding: "0 16px 4px",
  },
  logToggleBtn: {
    background: "none",
    border: "none",
    color: "#6f8792",
    fontSize: 11,
    cursor: "pointer",
    padding: "2px 0",
  },
  logPanel: {
    margin: "0 16px 12px",
    borderRadius: 8,
    border: "1px solid #1c2a31",
    background: "#0b1318",
    overflow: "hidden",
    maxHeight: 180,
  },
  logOutput: {
    margin: 0,
    padding: 10,
    fontSize: 11,
    fontFamily: "monospace",
    color: "#d7e0e5",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
    maxHeight: 160,
    overflowY: "auto",
  },
};
