import { useEffect, useState } from "react";
import {
  Database,
  Layers,
  Play,
  Plus,
  Search,
  Settings as SettingsIcon,
  Square,
  Trash2,
  Wifi,
} from "lucide-react";
import { api, onConfigsChanged, onPoolStatus, onTestProgress, onTestResult } from "./lib/api";
import type {
  ConfigsResponse,
  CoreInfo,
  ImportOutcome,
  ListQuery,
  PoolStatus,
  Settings,
  TestProgress,
} from "./types";

type View = "configs" | "pool" | "dead" | "settings";

const DEFAULT_QUERY: ListQuery = { search: "", status: "", protocol: "", sort: "addedAt-desc" };

export function App() {
  const [view, setView] = useState<View>("configs");
  const [data, setData] = useState<ConfigsResponse | null>(null);
  const [query, setQuery] = useState<ListQuery>(DEFAULT_QUERY);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [testing, setTesting] = useState(false);
  const [progress, setProgress] = useState<TestProgress | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [cores, setCores] = useState<CoreInfo[]>([]);
  const [pool, setPool] = useState<PoolStatus | null>(null);
  const [showImport, setShowImport] = useState(false);
  const [importText, setImportText] = useState("");
  const [subscriptionUrl, setSubscriptionUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [importResult, setImportResult] = useState<ImportOutcome | null>(null);

  async function refreshConfigs() {
    try {
      const response = await api.listConfigs(query);
      setData(response);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function refreshAll() {
    await Promise.all([
      refreshConfigs(),
      api.getSettings().then(setSettings).catch((e) => setError(String(e))),
      api.getCoreInfo().then(setCores).catch(() => {}),
      api.getPoolStatus().then(setPool).catch(() => {}),
    ]);
  }

  useEffect(() => {
    void refreshAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    void refreshConfigs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  useEffect(() => {
    const unlisten = [
      onConfigsChanged(() => void refreshConfigs()),
      onTestProgress((p) => {
        setProgress(p);
        setTesting(!p.finished);
        if (p.finished) {
          void refreshConfigs();
        }
      }),
      onTestResult((config) => {
        setData((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            configs: prev.configs.map((c) => (c.id === config.id ? config : c)),
          };
        });
      }),
      onPoolStatus((s) => setPool(s)),
    ];
    return () => {
      unlisten.forEach((fn) => void (fn as { unlisten?: () => void }).unlisten?.());
    };
  }, []);

  function toggleSelect(id: number) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function selectAllVisible() {
    setSelected((prev) => {
      const next = new Set(prev);
      (data?.configs ?? []).forEach((c) => next.add(c.id));
      return next;
    });
  }

  function clearSelection() {
    setSelected(new Set());
  }

  async function handleDelete(ids: number[]) {
    if (ids.length === 0) return;
    if (!window.confirm(`Delete ${ids.length} configuration(s)?`)) return;
    setBusy(true);
    try {
      await api.deleteConfigs(ids);
      setSelected(new Set());
      await refreshConfigs();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleTestSelected() {
    const ids = [...selected];
    if (ids.length === 0) return;
    setTesting(true);
    setProgress({ total: ids.length, done: 0, alive: 0, dead: 0, finished: false });
    try {
      await api.testConfigs(ids);
    } catch (e) {
      setTesting(false);
      setError(String(e));
    }
  }

  async function handleTestAll() {
    const ids = (data?.configs ?? []).map((c) => c.id);
    if (ids.length === 0) return;
    setTesting(true);
    setProgress({ total: ids.length, done: 0, alive: 0, dead: 0, finished: false });
    try {
      await api.testAll();
    } catch (e) {
      setTesting(false);
      setError(String(e));
    }
  }

  async function handleDeleteDead() {
    if (!window.confirm("Delete all dead/failed configurations?")) return;
    setBusy(true);
    try {
      await api.deleteDead();
      await refreshConfigs();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function doImport() {
    setBusy(true);
    setImportResult(null);
    try {
      let outcome: ImportOutcome;
      if (subscriptionUrl.trim()) {
        outcome = await api.importSubscription(subscriptionUrl.trim());
      } else {
        outcome = await api.importUris(importText);
      }
      setImportResult(outcome);
      await refreshConfigs();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleStartPool(mode: "internal" | "upstream") {
    setBusy(true);
    try {
      const status = await api.startPool(mode);
      setPool(status);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleStopPool() {
    setBusy(true);
    try {
      const status = await api.stopPool();
      setPool(status);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveSettings(next: Settings) {
    setBusy(true);
    try {
      const saved = await api.updateSettings(next);
      setSettings(saved);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const counts = data?.counts ?? { total: 0, alive: 0, dead: 0, untested: 0 };

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">Nex<span>Route</span></div>
        <NavItem view="configs" active={view} setView={setView} icon={<Database size={16} />} label="Configurations" />
        <NavItem view="pool" active={view} setView={setView} icon={<Layers size={16} />} label="Proxy Pool" />
        <NavItem view="dead" active={view} setView={setView} icon={<Trash2 size={16} />} label="Dead / Failed" />
        <NavItem view="settings" active={view} setView={setView} icon={<SettingsIcon size={16} />} label="Settings" />
      </aside>

      <main className="main">
        <header className="topbar">
          <h1>
            {view === "configs" && "Configuration List"}
            {view === "pool" && "Proxy Pool"}
            {view === "dead" && "Dead / Failed Configurations"}
            {view === "settings" && "Settings"}
          </h1>
          <div className="muted mono">{cores.length ? cores.map((c) => `${c.label} ${c.version ?? ""}`).join(" · ") : ""}</div>
        </header>

        <section className="content">
          {error && <div className="error">{error}</div>}

          {view === "configs" && (
            <ConfigsView
              data={data}
              query={query}
              setQuery={setQuery}
              selected={selected}
              toggleSelect={toggleSelect}
              selectAllVisible={selectAllVisible}
              clearSelection={clearSelection}
              testing={testing}
              progress={progress}
              counts={counts}
              onImport={() => setShowImport(true)}
              onTestAll={handleTestAll}
              onTestSelected={handleTestSelected}
              onDelete={handleDelete}
              busy={busy}
            />
          )}

          {view === "pool" && pool && (
            <PoolView
              pool={pool}
              onStart={handleStartPool}
              onStop={handleStopPool}
              busy={busy}
            />
          )}

          {view === "dead" && (
            <DeadView
              data={data}
              onDeleteDead={handleDeleteDead}
              onFilterDead={() => setQuery({ ...query, status: "dead" })}
              busy={busy}
            />
          )}

          {view === "settings" && settings && (
            <SettingsView settings={settings} cores={cores} onSave={saveSettings} busy={busy} />
          )}
        </section>
      </main>

      {showImport && (
        <div className="modal-backdrop" onClick={() => setShowImport(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Import Configurations</h2>
            <div className="form-row">
              <label>Proxy URIs (one per line)</label>
              <textarea value={importText} onChange={(e) => setImportText(e.target.value)} placeholder="vless://...&#10;vmess://...&#10;ss://..." />
            </div>
            <div className="form-row">
              <label>Or subscription URL</label>
              <input value={subscriptionUrl} onChange={(e) => setSubscriptionUrl(e.target.value)} placeholder="https://example.com/sub" />
            </div>
            {importResult && (
              <div className="muted" style={{ marginBottom: 12 }}>
                Added {importResult.added}, duplicates {importResult.duplicates}, invalid {importResult.invalid.length}.
                {importResult.invalid.length > 0 && (
                  <ul>
                    {importResult.invalid.map((i, idx) => (
                      <li key={idx} className="mono">{i.input}: {i.error}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
            <div className="toolbar">
              <button onClick={() => setShowImport(false)}>Close</button>
              <div className="spacer" />
              <button className="primary" onClick={() => void doImport()} disabled={busy}>
                <Plus size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
                Import
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

import type { ReactNode } from "react";
function NavItem(props: { view: View; active: View; setView: (v: View) => void; icon: ReactNode; label: string }) {
  return (
    <div
      className={`nav-item ${props.active === props.view ? "active" : ""}`}
      onClick={() => props.setView(props.view)}
    >
      {props.icon}
      {props.label}
    </div>
  );
}

/* ---------- Configs view ---------- */

interface ConfigsViewProps {
  data: ConfigsResponse | null;
  query: ListQuery;
  setQuery: (q: ListQuery) => void;
  selected: Set<number>;
  toggleSelect: (id: number) => void;
  selectAllVisible: () => void;
  clearSelection: () => void;
  testing: boolean;
  progress: TestProgress | null;
  counts: { total: number; alive: number; dead: number; untested: number };
  onImport: () => void;
  onTestAll: () => void;
  onTestSelected: () => void;
  onDelete: (ids: number[]) => void;
  busy: boolean;
}

function ConfigsView(props: ConfigsViewProps) {
  const configs = props.data?.configs ?? [];
  return (
    <>
      <div className="grid">
        <Stat label="Total" value={props.counts.total} />
        <Stat label="Alive" value={props.counts.alive} />
        <Stat label="Dead" value={props.counts.dead} />
        <Stat label="Untested" value={props.counts.untested} />
      </div>

      {props.testing && props.progress && (
        <div className="card" style={{ marginBottom: 14 }}>
          <div className="muted" style={{ marginBottom: 8 }}>
            Testing {props.progress.done}/{props.progress.total} — alive {props.progress.alive}, dead {props.progress.dead}
          </div>
          <div className="progress">
            <div style={{ width: `${(props.progress.done / Math.max(1, props.progress.total)) * 100}%` }} />
          </div>
        </div>
      )}

      <div className="toolbar">
        <div style={{ position: "relative", flex: 1, maxWidth: 320 }}>
          <Search size={14} style={{ position: "absolute", left: 10, top: 10, color: "var(--muted)" }} />
          <input
            style={{ paddingLeft: 32 }}
            placeholder="Search configurations"
            value={props.query.search}
            onChange={(e) => props.setQuery({ ...props.query, search: e.target.value })}
          />
        </div>
        <select value={props.query.status} onChange={(e) => props.setQuery({ ...props.query, status: e.target.value })} style={{ width: 130 }}>
          <option value="">All status</option>
          <option value="alive">Alive</option>
          <option value="dead">Dead</option>
          <option value="untested">Untested</option>
        </select>
        <select value={props.query.protocol} onChange={(e) => props.setQuery({ ...props.query, protocol: e.target.value })} style={{ width: 130 }}>
          <option value="">All protocols</option>
          <option value="vless">VLESS</option>
          <option value="vmess">VMess</option>
          <option value="ss">Shadowsocks</option>
          <option value="socks">SOCKS5</option>
          <option value="http">HTTP</option>
        </select>
        <select value={props.query.sort} onChange={(e) => props.setQuery({ ...props.query, sort: e.target.value })} style={{ width: 150 }}>
          <option value="addedAt-desc">Newest first</option>
          <option value="addedAt-asc">Oldest first</option>
          <option value="latency-asc">Latency ↑</option>
          <option value="latency-desc">Latency ↓</option>
          <option value="status-asc">Status</option>
        </select>
        <button onClick={props.onImport}>
          <Plus size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Import
        </button>
      </div>

      <div className="toolbar">
        <button onClick={props.selectAllVisible}>Select all</button>
        <button onClick={props.clearSelection}>Clear</button>
        <button onClick={props.onTestSelected} disabled={props.testing || props.selected.size === 0}>
          <Play size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Test Selected ({props.selected.size})
        </button>
        <button className="primary" onClick={props.onTestAll} disabled={props.testing || configs.length === 0}>
          <Play size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Test All
        </button>
        <div className="spacer" />
        <button className="danger" onClick={() => props.onDelete([...props.selected])} disabled={props.busy || props.selected.size === 0}>
          <Trash2 size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Delete Selected
        </button>
      </div>

      {configs.length === 0 ? (
        <div className="empty">No configurations yet. Import URIs or a subscription.</div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "hidden" }}>
          <table>
            <thead>
              <tr>
                <th style={{ width: 32 }}></th>
                <th>Protocol</th>
                <th>Name</th>
                <th>Server</th>
                <th>Transport</th>
                <th>Status</th>
                <th>Latency</th>
                <th>Last tested</th>
              </tr>
            </thead>
            <tbody>
              {configs.map((c) => (
                <tr key={c.id}>
                  <td>
                    <input
                      className="checkbox"
                      type="checkbox"
                      checked={props.selected.has(c.id)}
                      onChange={() => props.toggleSelect(c.id)}
                    />
                  </td>
                  <td><span className="badge untested" style={{ background: "transparent" }}>{c.protocolLabel}</span></td>
                  <td>
                    <div>{c.name}</div>
                    {c.lastError && <div className="muted" style={{ fontSize: 11 }}>{c.lastError}</div>}
                  </td>
                  <td className="mono">{c.server}:{c.port}</td>
                  <td>{c.transport}</td>
                  <td><span className={`badge ${c.status}`}>{c.status}</span></td>
                  <td className="mono">{c.lastRealDelayMs != null ? `${c.lastRealDelayMs} ms` : "—"}</td>
                  <td className="muted" style={{ fontSize: 12 }}>{c.lastTestedAt ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function Stat(props: { label: string; value: number }) {
  return (
    <div className="stat">
      <div className="label">{props.label}</div>
      <div className="value">{props.value}</div>
    </div>
  );
}

/* ---------- Pool view ---------- */

interface PoolViewProps {
  pool: PoolStatus;
  onStart: (mode: "internal" | "upstream") => void;
  onStop: () => void;
  busy: boolean;
}

function PoolView(props: PoolViewProps) {
  const p = props.pool;
  return (
    <>
      <div className="grid">
        <Stat label="Mode" value={0} />
      </div>
      <div className="card" style={{ marginBottom: 14 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
          <span className={`status-dot ${p.running ? "running" : "stopped"}`} />
          <strong>{p.running ? "Running" : "Stopped"}</strong>
          {p.mode && <span className="badge untested">{p.mode}</span>}
          {p.core && <span className="badge untested">{p.core}</span>}
          <div className="spacer" style={{ flex: 1 }} />
          {!p.running ? (
            <>
              <button className="primary" onClick={() => props.onStart("internal")} disabled={props.busy}>
                <Play size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
                Start Internal
              </button>
              <button onClick={() => props.onStart("upstream")} disabled={props.busy}>
                <Play size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
                Start Upstream Pool
              </button>
            </>
          ) : (
            <button className="danger" onClick={props.onStop} disabled={props.busy}>
              <Square size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
              Stop
            </button>
          )}
        </div>

        {p.running && p.mode === "internal" && (
          <div className="kv">
            <div className="k">HTTP endpoint</div>
            <div className="mono">{p.httpEndpoint}</div>
            <div className="k">SOCKS5 endpoint</div>
            <div className="mono">{p.socksEndpoint}</div>
            <div className="k">Alive backends</div>
            <div>{p.aliveCount}</div>
            <div className="k">Rotation</div>
            <div>{p.rotation}</div>
          </div>
        )}

        {p.running && p.endpoints.length > 0 && (
          <div style={{ marginTop: 12 }}>
            <div className="muted" style={{ marginBottom: 8 }}>Local endpoints (one pair per alive configuration)</div>
            <div className="endpoint-list">
              {p.endpoints.map((e) => (
                <div className="endpoint" key={e.configId}>
                  <span className="label">{e.name}</span>
                  <span className="muted">{e.protocol}</span>
                  <span>HTTP {e.http}</span>
                  <span>SOCKS {e.socks}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        {p.skipped.length > 0 && (
          <div style={{ marginTop: 12 }}>
            <div className="muted" style={{ marginBottom: 4 }}>Skipped</div>
            {p.skipped.map((s, i) => (
              <div key={i} className="muted" style={{ fontSize: 12 }}>{s.name}: {s.reason}</div>
            ))}
          </div>
        )}
      </div>

      {!p.running && (
        <div className="card">
          <p className="muted">The pool starts only after you press a button above. Mode A rotates requests internally across alive configurations through one HTTP and one SOCKS5 listener. Mode B exposes each alive configuration on its own listener pair for external routers.</p>
        </div>
      )}
    </>
  );
}

/* ---------- Dead view ---------- */

interface DeadViewProps {
  data: ConfigsResponse | null;
  onDeleteDead: () => void;
  onFilterDead: () => void;
  busy: boolean;
}

function DeadView(props: DeadViewProps) {
  const dead = (props.data?.configs ?? []).filter((c) => c.status === "dead");
  return (
    <>
      <div className="toolbar">
        <button className="danger" onClick={props.onDeleteDead} disabled={props.busy || dead.length === 0}>
          <Trash2 size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Delete Dead / Failed ({dead.length})
        </button>
      </div>
      {dead.length === 0 ? (
        <div className="empty">No dead or failed configurations.</div>
      ) : (
        <div className="card" style={{ padding: 0, overflow: "hidden" }}>
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Server</th>
                <th>Protocol</th>
                <th>Error</th>
                <th>Latency</th>
              </tr>
            </thead>
            <tbody>
              {dead.map((c) => (
                <tr key={c.id}>
                  <td>{c.name}</td>
                  <td className="mono">{c.server}:{c.port}</td>
                  <td>{c.protocolLabel}</td>
                  <td className="muted">{c.lastError ?? "—"}</td>
                  <td className="mono">{c.lastRealDelayMs != null ? `${c.lastRealDelayMs} ms` : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

/* ---------- Settings view ---------- */

interface SettingsViewProps {
  settings: Settings;
  cores: CoreInfo[];
  onSave: (s: Settings) => void;
  busy: boolean;
}

function SettingsView(props: SettingsViewProps) {
  const [draft, setDraft] = useState<Settings>(props.settings);
  useEffect(() => setDraft(props.settings), [props.settings]);
  return (
    <div className="card" style={{ maxWidth: 640 }}>
      <div className="form-row">
        <label>Test URL</label>
        <input value={draft.testUrl} onChange={(e) => setDraft({ ...draft, testUrl: e.target.value })} />
      </div>
      <div className="form-row">
        <label>Timeout (ms)</label>
        <input type="number" value={draft.timeoutMs} onChange={(e) => setDraft({ ...draft, timeoutMs: Number(e.target.value) })} />
      </div>
      <div className="form-row">
        <label>Core</label>
        <select value={draft.core} onChange={(e) => setDraft({ ...draft, core: e.target.value as Settings["core"] })}>
          {props.cores.map((c) => (
            <option key={c.id} value={c.id} disabled={!c.available}>
              {c.label} {c.version ? `(${c.version})` : ""} {c.available ? "" : "— unavailable"}
            </option>
          ))}
        </select>
      </div>
      <div className="form-row">
        <label>Rotation strategy</label>
        <select value={draft.rotation} onChange={(e) => setDraft({ ...draft, rotation: e.target.value as Settings["rotation"] })}>
          <option value="roundRobin">Round robin</option>
          <option value="random">Random</option>
        </select>
      </div>
      <div className="toolbar">
        <div className="spacer" />
        <button className="primary" onClick={() => props.onSave(draft)} disabled={props.busy}>
          <Wifi size={14} style={{ verticalAlign: "middle", marginRight: 6 }} />
          Save settings
        </button>
      </div>
      <div className="muted" style={{ fontSize: 12, marginTop: 12 }}>
        Real delay uses an actual HTTP request through a temporary local inbound. Dead configurations are never deleted automatically.
      </div>
    </div>
  );
}