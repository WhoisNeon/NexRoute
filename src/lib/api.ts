import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppInfo,
  ConfigsResponse,
  CoreInfo,
  ImportOutcome,
  ListQuery,
  PoolMode,
  PoolStatus,
  Settings,
  TestProgress,
  ConfigView,
} from "../types";

export const api = {
  listConfigs: (query: ListQuery) => invoke<ConfigsResponse>("list_configs", { query }),
  importUris: (text: string) => invoke<ImportOutcome>("import_uris", { text }),
  importSubscription: (url: string) => invoke<ImportOutcome>("import_subscription", { url }),
  deleteConfigs: (ids: number[]) => invoke<number>("delete_configs", { ids }),
  deleteDead: () => invoke<number>("delete_dead"),
  testConfigs: (ids: number[]) => invoke<number>("test_configs", { ids }),
  testAll: () => invoke<number>("test_all"),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  startPool: (mode: PoolMode) => invoke<PoolStatus>("start_pool", { mode }),
  stopPool: () => invoke<PoolStatus>("stop_pool"),
  getPoolStatus: () => invoke<PoolStatus>("get_pool_status"),
  getCoreInfo: () => invoke<CoreInfo[]>("get_core_info"),
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
};

// Backend PoolMode wire value is camelCase "Internal"/"Upstream"? Check: enum PoolMode has rename_all camelCase, variants Internal/Upstream => "internal"/"upstream".
// UI keeps display labels separately.

export const onConfigsChanged = (handler: () => void) =>
  listen("configs-changed", () => handler());
export const onTestProgress = (handler: (p: TestProgress) => void) =>
  listen<TestProgress>("test-progress", (e) => handler(e.payload));
export const onTestResult = (handler: (c: ConfigView) => void) =>
  listen<ConfigView>("test-result", (e) => handler(e.payload));
export const onPoolStatus = (handler: (s: PoolStatus) => void) =>
  listen<PoolStatus>("pool-status", (e) => handler(e.payload));