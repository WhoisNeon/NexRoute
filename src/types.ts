export type ConfigStatus = "untested" | "alive" | "dead";

export interface ConfigView {
  id: number;
  protocol: string;
  protocolLabel: string;
  name: string;
  server: string;
  port: number;
  transport: string;
  security: string;
  rawUri: string;
  addedAt: string;
  lastTestedAt: string | null;
  lastRealDelayMs: number | null;
  status: ConfigStatus;
  lastError: string | null;
}

export interface Counts {
  total: number;
  alive: number;
  dead: number;
  untested: number;
}

export interface ConfigsResponse {
  configs: ConfigView[];
  counts: Counts;
}

export interface InvalidUri {
  input: string;
  error: string;
}

export interface ImportOutcome {
  added: number;
  duplicates: number;
  invalid: InvalidUri[];
}

export interface ListQuery {
  search: string;
  status: string;
  protocol: string;
  sort: string;
}

export type CoreKind = "sing-box" | "xray";
export type Rotation = "roundRobin" | "random";
export type PoolMode = "internal" | "upstream";

export interface Settings {
  testUrl: string;
  timeoutMs: number;
  core: CoreKind;
  rotation: Rotation;
}

export interface PoolEndpoint {
  configId: number;
  name: string;
  protocol: string;
  http: string;
  socks: string;
}

export interface SkippedConfig {
  name: string;
  reason: string;
}

export interface PoolStatus {
  running: boolean;
  mode: PoolMode | null;
  core: CoreKind | null;
  httpEndpoint: string | null;
  socksEndpoint: string | null;
  endpoints: PoolEndpoint[];
  aliveCount: number;
  skipped: SkippedConfig[];
  rotation: Rotation;
  startedAt: string | null;
}

export interface CoreInfo {
  id: CoreKind;
  label: string;
  available: boolean;
  version: string | null;
  path: string | null;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  os: string;
  arch: string;
}

export interface TestProgress {
  total: number;
  done: number;
  alive: number;
  dead: number;
  finished: boolean;
}