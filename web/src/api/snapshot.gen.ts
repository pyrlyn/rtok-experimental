// Generated from ws.schema.json by scripts/gen-api.mjs (T310.2). Do not edit: change the Rust
// types in src/web, bless the schema, then run `npm run gen:api`.

/**
 * A message a client sends over `/ws`.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ClientMessage".
 */
export type ClientMessage =
  | {
      expand: string;
    }
  | {
      set: SetRequest;
    }
  | {
      project: ProjectRequest;
    }
  | {
      doctor: DoctorRequest;
    }
  | {
      junk: JunkRequest;
    }
  | {
      graph: DrillRequest;
    }
  | {
      diff: DiffRequest;
    }
  | {
      export: ExportRequest;
    }
  | {
      import: ImportRequest;
    }
  | {
      calls: CallsRequest;
    };
/**
 * The registry writes the graph page offers; `<project>` is an id or a
 * root path, as in `rtok graph projects`.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ProjectRequest".
 */
export type ProjectRequest =
  | {
      action: "select";
      project: string;
    }
  | {
      action: "link";
      both: boolean;
      from: string;
      to: string;
    }
  | {
      action: "unlink";
      both: boolean;
      from: string;
      to: string;
    };
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DoctorAction".
 */
export type DoctorAction = "plan" | "apply";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "JunkAction".
 */
export type JunkAction = "plan" | "apply";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Format".
 */
export type Format = "json" | "svg" | "png";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Level2".
 */
export type Level2 = "overview" | "symbols";
/**
 * A frame the server pushes besides the [`Snapshot`] itself.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ServerFrame".
 */
export type ServerFrame =
  | {
      text: string;
      type: "message";
    }
  | {
      id: string;
      text: string;
      type: "expand";
    }
  | {
      plan: Plan;
      type: "doctorplan";
    }
  | {
      fixed: Fixed;
      type: "doctorfixed";
    }
  | {
      plan: Cleared;
      type: "junkplan";
    }
  | {
      cleared: Cleared;
      type: "junkcleared";
    }
  | {
      graph: DrillGraph;
      type: "graph";
    }
  | {
      diff: DiffReport;
      project: string;
      type: "diff";
    }
  | {
      file: ExportFile;
      type: "export";
    }
  | {
      export: Export;
      name: string;
      type: "imported";
    }
  | {
      calls: CallsView;
      type: "calls";
    };
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillEdgeKind".
 */
export type DrillEdgeKind = "contains" | "calls" | "implements" | "imports";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillNodeKind".
 */
export type DrillNodeKind = ("file" | "type" | "module" | "function") | "external";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ModuleState".
 */
export type ModuleState = "installed" | "not_installed" | "not_supported";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Kind".
 */
export type Kind = "missing" | "unreachable" | "backend_down" | "link_broken";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Component".
 */
export type Component = "freshness" | "backend" | "links";
/**
 * Whether Claude Code defers MCP tools (T388).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ToolSearchState".
 */
export type ToolSearchState = ("enabled" | "disabled") | "unknown";
/**
 * The mode that answers a project's requests.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Chosen".
 */
export type Chosen = "lsp" | "tags";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Level".
 */
export type Level = "missing" | "good" | "warn" | "bad" | "indexing";
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "LinkKind".
 */
export type LinkKind = "manual" | "auto";
/**
 * How a project got into the registry (T329 §1).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Origin".
 */
export type Origin = "manual" | "session" | "worktree" | "mcp" | "reference";
/**
 * The board's columns, the four statuses of [`Status`] under their `rtok task` names.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TaskColumn".
 */
export type TaskColumn = "open" | "in-progress" | "done" | "closed";
/**
 * Where the Tasks page stands. `Unconfigured` is an ordinary case (the project never ran
 * `rtok task init`), not a failure, so the page shows how to start instead of an error.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TasksState".
 */
export type TasksState = "ready" | "reading" | "unconfigured" | "failed";

/**
 * Root of the schema: one property per direction, so every type lands in `$defs` once.
 */
export interface WsProtocol {
  client: ClientMessage;
  server: ServerFrame;
  snapshot: Snapshot;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SetRequest".
 */
export interface SetRequest {
  key: string;
  value: boolean;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DoctorRequest".
 */
export interface DoctorRequest {
  action: DoctorAction;
  selection: Selection;
}
/**
 * What the user changed since the defaults.
 */
export interface Selection {
  /**
   * Entries made the kept copy of their duplicate.
   */
  keep: Ref[];
  /**
   * Entries whose selection is the opposite of their default.
   */
  toggled: Ref[];
}
/**
 * An entry as the page names it: the file and the key path inside it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Ref".
 */
export interface Ref {
  path: string;
  source: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "JunkRequest".
 */
export interface JunkRequest {
  action: JunkAction;
  /**
   * For `apply`: the paths of the plan the user was shown. A planned item not named here
   * stays, so nothing that appeared since the plan goes unseen.
   */
  paths: string[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillRequest".
 */
export interface DrillRequest {
  depth: number | null;
  /**
   * Files shown as their definitions.
   */
  expand: string[];
  /**
   * A symbol shown with its callers and callees.
   */
  focus: DrillFocus | null;
  /**
   * Nodes shown before "+N more"; the page raises it on a click.
   */
  limit: number | null;
  /**
   * An id or a root path, as in `rtok graph projects`.
   */
  project: string;
  /**
   * Finds symbols in the project and its scope.
   */
  query: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillFocus".
 */
export interface DrillFocus {
  name: string;
  path: string;
}
/**
 * What the graph page's Compare mode asks (T329.35). The old side is never a path: the page
 * sends the text of a saved export it opened itself, because the websocket answers anything on
 * localhost and must not be made to read a file somebody else chose.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffRequest".
 */
export interface DiffRequest {
  export: DiffExport | null;
  /**
   * As `rtok graph diff --from`: a ref, or `PROJECT:REF`; none is `HEAD`.
   */
  from: string[];
  /**
   * An id or a root path, as in `rtok graph projects`.
   */
  project: string;
  /**
   * A ref; none is the working tree.
   */
  to: string | null;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffExport".
 */
export interface DiffExport {
  /**
   * The file's name, for the answer's `from`.
   */
  name: string;
  /**
   * The file's content.
   */
  text: string;
}
/**
 * A download from the graph page's Export menu (T329.40): the file `rtok graph export` writes for
 * the same arguments.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ExportRequest".
 */
export interface ExportRequest {
  depth: number | null;
  /**
   * A symbol name; it wins over `level`, as `--focus` does.
   */
  focus: string | null;
  format: Format;
  level: Level2;
  /**
   * An id or a root path, as in `rtok graph projects`.
   */
  project: string;
  /**
   * PNG only: 1 to 4 times the SVG size.
   */
  scale: number | null;
  transparent: boolean;
}
/**
 * A saved JSON export the page read itself, to show read-only. Like Compare mode, it never names
 * a path: the websocket answers anything on localhost, so the file arrives as its text.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ImportRequest".
 */
export interface ImportRequest {
  /**
   * The file's name, for the banner and the errors.
   */
  name: string;
  text: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "CallsRequest".
 */
export interface CallsRequest {
  subscribe: boolean;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Plan".
 */
export interface Plan {
  /**
   * The diff of every file the selection would change.
   */
  diff: string;
  items: Item[];
  /**
   * Selected entries the engine will not remove, with why.
   */
  refused: string[];
}
/**
 * One line of the checklist.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Item".
 */
export interface Item {
  agent: string;
  /**
   * Whether this copy may be made the kept one.
   */
  can_keep: boolean;
  detail: string;
  /**
   * The file that holds the copy kept instead, for a duplicate.
   */
  kept_in: string | null;
  kind: string;
  /**
   * What the entry runs or is called.
   */
  label: string;
  path: string;
  selected: boolean;
  /**
   * In a project's shared file: starts unselected.
   */
  shared: boolean;
  source: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Fixed".
 */
export interface Fixed {
  /**
   * 1 when a selected entry was skipped or failed.
   */
  code: number;
  text: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Cleared".
 */
export interface Cleared {
  freed_bytes: number;
  items: Planned[];
  planned_bytes: number;
  yes: boolean;
}
/**
 * One item of the plan and what happened to it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Planned".
 */
export interface Planned {
  /**
   * `clear` (removed, or would be on a dry run) or `skip`.
   */
  action: string;
  agent: string;
  bytes: number;
  /**
   * Planned and not removed: the re-check refused it or the removal failed.
   */
  failed: boolean;
  /**
   * Files in it, counted for a planned item before anything is removed.
   */
  files: number;
  kind: string;
  /**
   * Unix seconds of the newest file in it; `None` when unreadable (T330.6).
   */
  last_used: number | null;
  note: string;
  path: string;
  /**
   * In the plan: false for an item left at planning time (a running agent's).
   */
  planned: boolean;
  /**
   * Why it is junk, as `list` says it.
   */
  reason: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillGraph".
 */
export interface DrillGraph {
  edges: DrillEdge[];
  hits: DrillHit[];
  /**
   * Nodes left out by the cap.
   */
  more: number;
  name: string;
  nodes: DrillNode[];
  /**
   * Some files changed since the last index run, so the picture lags the tree.
   */
  partial: boolean;
  project: number;
  root: string;
  /**
   * `ok`, `not indexed` or `missing`.
   */
  state: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillEdge".
 */
export interface DrillEdge {
  count: number;
  from: string;
  kind: DrillEdgeKind;
  to: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillHit".
 */
export interface DrillHit {
  kind: string;
  line: number;
  name: string;
  path: string;
  project: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DrillNode".
 */
export interface DrillNode {
  id: string;
  kind: DrillNodeKind;
  label: string;
  line: number;
  path: string;
  /**
   * The project that owns the node: the one asked for, or the linked one for `external`.
   */
  project: number;
  signature: string;
  /**
   * The file changed since the last index run.
   */
  stale: boolean;
  /**
   * Edge count through the node; the page sizes it by this.
   */
  weight: number;
}
/**
 * `--json` and the graph page's `diff` frame: one typed shape, so the page never recomputes or
 * re-parses what the CLI prints.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffReport".
 */
export interface DiffReport {
  from: string;
  links_added: DiffLink[];
  links_removed: DiffLink[];
  /**
   * Linked projects that could not answer, and what an export cannot compare.
   */
  notes?: string;
  projects: DiffProject[];
  to: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffLink".
 */
export interface DiffLink {
  from: string;
  kind: string;
  to: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffProject".
 */
export interface DiffProject {
  added: DiffDef[];
  changed: DiffDef[];
  edges_added: DiffEdge[];
  edges_removed: DiffEdge[];
  from: string;
  /**
   * Rows left out of the lists by [`DiffReport::capped`].
   */
  more?: number;
  moved: DiffMove[];
  not_analysed: DiffUnread[];
  project: string;
  removed: DiffDef[];
  renamed: DiffMove[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffDef".
 */
export interface DiffDef {
  /**
   * `changed` and `removed` rows only.
   */
  callers?: string[] | null;
  kind: string;
  line: number;
  name: string;
  path: string;
  /**
   * `changed` rows only: the signature moved, not just the body.
   */
  signature_changed?: boolean | null;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffEdge".
 */
export interface DiffEdge {
  name: string;
  path: string;
  scope: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffMove".
 */
export interface DiffMove {
  from: DiffDef;
  to: DiffDef;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "DiffUnread".
 */
export interface DiffUnread {
  path: string;
  reason: string;
}
/**
 * A finished download. The bytes travel as base64 because a PNG is not text and the frame is.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ExportFile".
 */
export interface ExportFile {
  data: string;
  mime: string;
  name: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Export".
 */
export interface Export {
  edges: Edge[];
  links: Link[];
  meta: Meta;
  /**
   * Empty at the `overview` level.
   */
  nodes: Node[];
  projects: Proj[];
  schema: "rtok.graph.v1";
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Edge".
 */
export interface Edge {
  from: string;
  kind: string;
  to: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Link".
 */
export interface Link {
  from: number;
  kind: string;
  reason?: string | null;
  /**
   * Call references from `from` into `to` found in the indexes.
   */
  references: number;
  to: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Meta".
 */
export interface Meta {
  depth?: number | null;
  exported_at: number;
  focus?: string | null;
  /**
   * `overview`, `symbols` or `focus`.
   */
  level: string;
  notes: string[];
  /**
   * A project was still indexing, not indexed, or could not answer.
   */
  partial: boolean;
  redacted: boolean;
  rtok_version: string;
  /**
   * Project names of the scope.
   */
  scope: string[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Node".
 */
export interface Node {
  id: string;
  kind: string;
  line: number;
  name: string;
  /**
   * Relative to the project root.
   */
  path: string;
  project: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Proj".
 */
export interface Proj {
  /**
   * `tags`, `lsp` or `text`: the mode that answers for the project.
   */
  backend: string;
  /**
   * `ok`, `stale`, `not indexed`, `missing` or `unknown`.
   */
  health: string;
  /**
   * 0 for a directory that is not in the registry.
   */
  id: number;
  indexed_at?: number | null;
  name: string;
  origin: string;
  /**
   * `~/...` under the home directory, `.../name` elsewhere, unless redaction is off.
   */
  root: string;
}
/**
 * The state of the live calls panel at `now`.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "CallsView".
 */
export interface CallsView {
  /**
   * Newest first.
   */
  feed: Finished[];
  /**
   * `[plugins.graph] live_heat_window_s`: how long a node stays warm on the live canvas.
   */
  heat_window_s: number;
  /**
   * The server's clock in epoch milliseconds: the page ages rows by `now - at` on this clock,
   * so a skew between the two machines moves nothing.
   */
  now: number;
  running: Running[];
  /**
   * One per chip, in the order of [`WINDOWS`]; the last is "since start".
   */
  windows: WindowView[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Finished".
 */
export interface Finished {
  after: number;
  at: number;
  backend: string | null;
  before: number;
  call: string;
  caller: string;
  error: string | null;
  interrupted: boolean;
  ms: number | null;
  ok: boolean;
  project: string | null;
  session: string;
  target: string | null;
  tool: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Running".
 */
export interface Running {
  /**
   * The reader's clock: the interrupt timeout must not depend on the writer's.
   */
  at: number;
  call: string;
  /**
   * T329.34: what the store calls the session (`claude 3f9a1c2e`), else the session's start.
   */
  caller: string;
  project: string | null;
  session: string;
  target: string | null;
  tool: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "WindowView".
 */
export interface WindowView {
  after: number;
  backends: {
    [k: string]: number;
  };
  before: number;
  calls: number;
  caps: number;
  crossed: number;
  failed: number;
  fallbacks: number;
  files_touched: number;
  label: string;
  /**
   * `None` before any call in the window has ended.
   */
  latency: Latency | null;
  projects_hit: number;
  spark: Spark;
  symbols: number;
  symbols_returned: number;
  /**
   * Most calls first.
   */
  tools: ToolView[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Latency".
 */
export interface Latency {
  p50: number;
  p95: number;
}
/**
 * Calls and tokens saved per slot over a window, oldest first. Summed from the same buckets as
 * the window totals, so the slots add up to them.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Spark".
 */
export interface Spark {
  calls: number[];
  saved: number[];
  /**
   * What the slots cover. "since start" reads the buckets, which are kept for 15 minutes only.
   */
  span_ms: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ToolView".
 */
export interface ToolView {
  calls: number;
  saved: number;
  tool: string;
}
/**
 * Everything a surface needs for one refresh. `Default` is the empty frame a surface
 * paints while its first read is still running.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Snapshot".
 */
export interface Snapshot {
  agent_usage: UsagePage;
  /**
   * Calls page (T15.5): the last [`CALLS_ROWS`] ledger rows, newest first.
   */
  calls: CallRow[];
  /**
   * Config page (T228): `rtok config show --sources`'s rows, through
   * [`config_page_text`] (D27, no second layering). `None` on a failed tick.
   */
  config: string | null;
  /**
   * Doctor page (T15.6): what `rtok doctor` reports — hooks, MCP servers, proxy
   * chains. `None` when this tick's probes failed; the page renders the failure
   * rather than zeros, the way an unreadable store renders an empty page.
   */
  doctor: Report | null;
  /**
   * Set when the store will not open or this tick's doctor probe failed (T60.6). Both
   * surfaces render it as a banner instead of an empty page.
   */
  error?: string | null;
  /**
   * Graph page (T230): `graph status`'s index health (rows, files, the T68.3
   * pending/staleness set, `indexed_at`) plus `graph dead`'s unreferenced-definition
   * list, folded into one page (D27) — [`graph_page_text`], read from the store on
   * this tick (no re-indexing). `None` when the `graph` feature is off or the store
   * read failed.
   */
  graph: string | null;
  /**
   * Hosts page (T231): `agents list`'s blocks — kind, detected version, installed
   * surfaces, config path — one per known host variant (D27), so `agents list` /
   * `agents info` can join `COMMAND_PAGES`. [`hosts_page_text`] reuses the same
   * probe `agents_list`'s JSON form calls (T168's `--version` noise filter and all)
   * behind a cache: never blocks a 2 s tick on a cold or stale probe — the tick
   * renders the last known text, or "probing hosts…" before the first one lands.
   */
  hosts: string;
  /**
   * The Hosts page's junk card (T330.7): the numbers of the `junk` section of `hosts`, as data
   * ([`junk_page`] reads the one cached report). `None` while the first measurement runs.
   */
  junk: JunkCard | null;
  /**
   * Logs page (T15.7): the last `[log] lines` log lines, newest first — the same
   * selection `rtok logs` screens ([`Model::log_lines`], T15.11). Riding the snapshot
   * makes the page both surfaces' (D23); `[log] lines` is the frame's bound too.
   */
  logs: string[];
  /**
   * Plugins page: one entry per catalogue plugin.
   */
  plugins: PluginPage[];
  /**
   * Project registry (T329.12): every registered project with its index state and links,
   * the same rows `rtok graph projects --json` prints. `None` when the `graph` feature is
   * off or the store read failed.
   */
  projects: ProjectRow[] | null;
  /**
   * Archive ids keyed by `calls[].id` (T60.4). Both surfaces read this map; neither
   * queries the store for an expand handle (D23 / D27).
   */
  ref_ids: {
    /**
     * This interface was referenced by `undefined`'s JSON-Schema definition
     * via the `patternProperty` "^-?\d+$".
     */
    [k: string]: string;
  };
  /**
   * Services page (T229): `demon status`'s per-service rows plus `otel status`'s
   * exporter health, through [`services_page_text`] (D27, no second reader —
   * [`Model::demon`] and [`otel_status`] already build both). `None` on a failed
   * tick.
   */
  services: string | null;
  /**
   * Sessions page (T25.1): one row per session, newest first.
   */
  sessions: SessionTotals[];
  skills: SkillsPage;
  /**
   * Stats page (T227): `stats --price`'s table plus `stats --cache`'s table, folded
   * into one page (D27) — [`stats_page_text`], built from the same transcript scan
   * [`stats_skills`] already runs for the Skills page (no second aggregation). `None`
   * when this tick's transcripts read failed.
   */
  stats: string | null;
  tasks: TasksPage;
  type: string;
  usage: Overview;
  /**
   * Worktrees page (T232): `worktree list`'s rows — path, branch, owner, age,
   * `target/` size and state — through [`worktrees_page_text`], the same
   * [`crate::worktree::list::rows`]/[`crate::worktree::list::to_table`] `worktree
   * list` already calls (D27, no second reader or directory walk); `gc`/`clean`
   * stay CLI-only verdicts. `None` only when the current directory is unreadable.
   */
  worktrees: string | null;
}
/**
 * Usage page (T358.5): what `rtok agents usage` reports, through [`usage_page`]. The
 * overview's own `usage` key is the proxy's totals, so this one carries the page's name
 * in its own words.
 */
export interface UsagePage {
  report: Report2 | null;
  text: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Report2".
 */
export interface Report2 {
  agents: Agent[];
  /**
   * `--by model` only: the model rows the middle table shows instead of `agents`.
   */
  models?: ModelRow[] | null;
  periods: Period[];
  /**
   * Hosts whose files exist but could not be read: named, counted nowhere.
   */
  skipped: Skipped[];
  source: string;
  /**
   * The last day with usage, in `tz`.
   */
  through: string | null;
  totals: Totals;
  tz: string;
  unpriced: Unpriced[];
  /**
   * Distinct model ids without a price.
   */
  unpriced_models: number;
}
/**
 * Token legs and the estimated cost of one table row. `cost_usd` is `None` when no model in
 * the row has a price: `-` in the table, `null` in JSON, never `$0.00` (that means free).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Agent".
 */
export interface Agent {
  cache_read: number;
  cache_write: number;
  cost_usd: number | null;
  /**
   * `both` only: `through_rtok_tokens` over the logs' tokens, so an agent that bypasses
   * the proxy reads low. `None` when the logs hold no tokens for it.
   */
  coverage?: number | null;
  host: string;
  input: number;
  name: string;
  output: number;
  saved_tokens?: number;
  saved_usd?: number | null;
  /**
   * `both` only: tokens of this agent that also passed through rtok.
   */
  through_rtok_tokens?: number | null;
  tokens: number;
}
/**
 * Token legs and the estimated cost of one table row. `cost_usd` is `None` when no model in
 * the row has a price: `-` in the table, `null` in JSON, never `$0.00` (that means free).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ModelRow".
 */
export interface ModelRow {
  cache_read: number;
  cache_write: number;
  cost_usd: number | null;
  input: number;
  model: string;
  output: number;
  tokens: number;
}
/**
 * Token legs and the estimated cost of one table row. `cost_usd` is `None` when no model in
 * the row has a price: `-` in the table, `null` in JSON, never `$0.00` (that means free).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Period".
 */
export interface Period {
  cache_read: number;
  cache_write: number;
  cost_usd: number | null;
  input: number;
  output: number;
  period: string;
  tokens: number;
}
/**
 * A host whose session files exist but could not be read: named once, counted nowhere.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Skipped".
 */
export interface Skipped {
  host: string;
  path: string;
  reason: string;
}
/**
 * Token legs and the estimated cost of one table row. `cost_usd` is `None` when no model in
 * the row has a price: `-` in the table, `null` in JSON, never `$0.00` (that means free).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Totals".
 */
export interface Totals {
  cache_read: number;
  cache_write: number;
  cost_usd: number | null;
  /**
   * Distinct `(agent, day)` pairs with usage, what ccusage calls daily rows.
   */
  daily_rows: number;
  input: number;
  output: number;
  saved_tokens?: number;
  saved_usd?: number | null;
  /**
   * Distinct session ids.
   */
  sessions: number;
  tokens: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Unpriced".
 */
export interface Unpriced {
  host: string;
  model: string;
  tokens: number;
}
/**
 * One `calls` row as the Calls page serves it ([`Store::recent_calls`], T15.5): the
 * ledger's own columns plus the slugs its ids point at and — when the call is one
 * that recorded usage — the newest `usage` row linked to it. One query's output, so
 * no renderer can re-derive a field differently (D27); `api` `None` means no usage
 * row is linked (a hook, MCP call or plugin run carries none).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "CallRow".
 */
export interface CallRow {
  api: string | null;
  cache_create: number | null;
  cache_read: number | null;
  error: string | null;
  host: string | null;
  id: number;
  input: number | null;
  kind: string;
  model: string | null;
  ms: number | null;
  name: string | null;
  ok: number;
  output: number | null;
  parent_id: number | null;
  plugin: string | null;
  provider: string | null;
  session: string;
  surface: string;
  ts: number;
}
/**
 * What `rtok doctor` found, as data.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Report".
 */
export interface Report {
  /**
   * Every host variant and the state of each rtok module in it, as `agent setup` prints.
   */
  agents: AgentModules[];
  auto_compact_window: string | null;
  bash_max_output_length: string | null;
  /**
   * What the graph health check raised for linked projects: missing, unreachable, backend
   * down, link broken (T329.17). Read from the store the checking processes write.
   */
  graph_alerts?: Alert[];
  /**
   * Projects whose graph health score is under 80, weakest first, with reasons and fixes
   * (T329.19).
   */
  graph_health?: Weak[];
  hooks_by_event: {
    [k: string]: number;
  };
  hooks_total: number;
  /**
   * The instruction audit (T7.2): `Some` only when `[doctor] instructions` ran — an audit
   * that found nothing still prints its section header, as it always did.
   */
  instructions: Instructions | null;
  /**
   * One probed MCP server: name, command, tool count, description tokens.
   */
  mcp: ServerInfo[];
  mcp_tool_search: ToolSearch;
  /**
   * MCP tool search is not confirmed on: `mcp_tool_search.state` is `disabled`, or `unknown`
   * with a custom `ANTHROPIC_BASE_URL` (the heuristic).
   */
  mcp_tool_search_disabled: boolean;
  /**
   * Host-native features that duplicate a running rtok surface (T59.7), each
   * naming the rtok config key that turns the duplicate side off. Advice: the
   * lines say "duplicate", never "saves N".
   */
  overlaps?: string[];
  /**
   * Hooks that lead nowhere or cannot be checked (T331.1); the list later detectors extend.
   */
  problems?: Problem[];
  /**
   * The proxy chain behind `ANTHROPIC_BASE_URL`, hops joined with `→`.
   */
  proxy: string;
  /**
   * The proxy chain behind the OpenAI seed (`OPENAI_BASE_URL` or a host config).
   */
  proxy_openai: string;
  /**
   * Read-class token share from the transcripts (`None` = no data, fail open).
   */
  read_share?: ReadShare | null;
  /**
   * The skills audit (T61.3): `Some` when the probe ran; an empty tree prints no section.
   */
  skills?: SkillsAudit | null;
  /**
   * Advice for enabling `[proxy.tools_rewrite]` when applicable (T59.5).
   */
  tools_rewrite_advice?: string | null;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "AgentModules".
 */
export interface AgentModules {
  host: string;
  kind: string;
  modules: ModuleRow[];
}
/**
 * One [`MODULES`] row of a host variant: its state and the note printed after it — the flag
 * that would install it, or the reason it cannot be installed.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ModuleRow".
 */
export interface ModuleRow {
  name: string;
  note: string;
  state: ModuleState;
}
/**
 * One raised alert, as stored and as `rtok graph projects --json` and `rtok doctor --json`
 * print it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Alert".
 */
export interface Alert {
  detail: string;
  kind: Kind;
  project: string;
  root: string;
  /**
   * Unix seconds of the check that raised it.
   */
  since: number;
}
/**
 * One project under [`NOTICE_BELOW`], as `rtok doctor` lists it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Weak".
 */
export interface Weak {
  project: string;
  reasons: Reason[];
  root: string;
  score: number;
}
/**
 * What lowered a component, and what to do about it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Reason".
 */
export interface Reason {
  component: Component;
  fix: string;
  text: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Instructions".
 */
export interface Instructions {
  duplicates: [unknown, unknown][];
  rows: InstructionRow[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "InstructionRow".
 */
export interface InstructionRow {
  name: string;
  path: string;
  tokens: number;
  warn: boolean;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ServerInfo".
 */
export interface ServerInfo {
  cmd: string;
  desc_tokens: number;
  name: string;
  tools: number;
}
/**
 * The tool-search state and where it came from; `source` is `None` for the default (nothing to
 * report), so a plain install prints no line.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ToolSearch".
 */
export interface ToolSearch {
  source?: string | null;
  state: ToolSearchState;
}
/**
 * One finding of a doctor config check. The list is shared by every T331 detector.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Problem".
 */
export interface Problem {
  agent: string;
  /**
   * The command as written, never expanded.
   */
  command?: string;
  detail: string;
  event?: string;
  /**
   * Whether `doctor --fix` may remove it (T331.5): only `broken-hook`.
   */
  fixable: boolean;
  /**
   * Copies of one duplicate share a `group`; `keep` marks the copy to keep (T331.3).
   */
  group?: number | null;
  keep?: boolean;
  /**
   * `broken-hook`, `suspect-hook`, `unverified-hook`, `duplicate-hook`, `duplicate-mcp`,
   * `conflicting-mcp`, `own-mcp`, `stale-plugin` or `unreadable-config`.
   */
  kind: string;
  matcher?: string | null;
  /**
   * The key path of the entry inside `source`.
   */
  path: string;
  /**
   * The config file the entry lives in.
   */
  source: string;
}
/**
 * Share of Read-class transcript tokens spent in native Grep/Glob (T50.4):
 * `(grep + glob) / (read + grep + glob)` by estimated tokens. `None` when the
 * transcripts hold no Read-class results — the default stays off on no data.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ReadShare".
 */
export interface ReadShare {
  glob_tokens: number;
  grep_tokens: number;
  read_tokens: number;
  share: number;
}
/**
 * The skills audit (T61.3): what the host lists and what it costs the system
 * prompt. Advice only — nothing here edits a file.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillsAudit".
 */
export interface SkillsAudit {
  /**
   * Description bytes the listing rides with every request (≈ tokens/4).
   */
  desc_bytes: number;
  /**
   * Names reachable from more than one real path (T392). Advice only: rtok never deletes a
   * skill it does not own.
   */
  duplicates?: SkillDuplicate[];
  /**
   * Skills loaded again with no compaction between, from the transcripts (T392).
   */
  repeats?: SkillRepeat[];
  rows: SkillRow[];
}
/**
 * One skill name listed from several places (T392).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillDuplicate".
 */
export interface SkillDuplicate {
  /**
   * `(source, directory)` of every listing, in root order.
   */
  copies: [unknown, unknown][];
  /**
   * Description tokens the extra listings add to every request.
   */
  extra_tokens: number;
  /**
   * Every body is the same text; a differing body is the case the model cannot tell apart.
   */
  identical: boolean;
  name: string;
}
/**
 * A skill body re-sent into the same context window (T392).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillRepeat".
 */
export interface SkillRepeat {
  loads: number;
  name: string;
  /**
   * Estimated tokens of the repeated bodies.
   */
  tokens: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillRow".
 */
export interface SkillRow {
  body_bytes: number;
  desc_chars: number;
  /**
   * Invocations in the last 30 d from the T61.1 fold; `None` = no data.
   */
  invocations?: number | null;
  name: string;
  /**
   * `user`, `project`, or `plugin:<id>@<marketplace>`.
   */
  source: string;
  /**
   * Body over 8 KB — almost always a `references/` candidate.
   */
  warn_body: boolean;
  /**
   * `description:` over [`SKILL_DESC_MAX`], the cap rtok's own skills follow.
   */
  warn_desc: boolean;
  /**
   * Listed but never invoked in the window (only when data exists).
   */
  warn_never: boolean;
}
/**
 * The `junk` card of the Hosts page.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "JunkCard".
 */
export interface JunkCard {
  agents: CardAgent[];
  freed_default_bytes: number;
  freed_review_bytes: number;
  total_bytes: number;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "CardAgent".
 */
export interface CardAgent {
  /**
   * "Freed by `clear`".
   */
  freed_default_bytes: number;
  /**
   * "Freed with `--include review`".
   */
  freed_review_bytes: number;
  kinds: CardKind[];
  name: string;
  total_bytes: number;
}
/**
 * One junk kind under an agent.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "CardKind".
 */
export interface CardKind {
  /**
   * `safe`, `review`, `explicit` or `never` (the T330 classes).
   */
  class: string;
  items: number;
  kind: string;
  size_bytes: number;
}
/**
 * A plugin's page: its manifest, the static copy it contributes through
 * `Plugin::dashboard_page`, and the stats widget when it saves tokens.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "PluginPage".
 */
export interface PluginPage {
  enabled: boolean;
  fields: [unknown, unknown][];
  id: string;
  saves_tokens: boolean;
  stats: Stats | null;
  summary: string;
  surfaces: string[];
  title: string;
}
/**
 * The shared stats widget: `usage` rows for the overview, `Measurement` rows per plugin.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Stats".
 */
export interface Stats {
  cache_create: number;
  cache_read: number;
  est_after: number;
  est_before: number;
  input: number;
  output: number;
  rows: number;
}
/**
 * One registry row as `graph projects` prints it and the `/ws` snapshot carries it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ProjectRow".
 */
export interface ProjectRow {
  /**
   * What the health check raised for this project (T329.17); absent while nothing is wrong.
   */
  alerts?: Alert[];
  /**
   * Which graph mode works here, as the last process to answer for it recorded (T329.11);
   * absent until a request under `lsp` or `auto` has checked.
   */
  backend?: Capability | null;
  created_at: number;
  health: Score;
  id: number;
  index: ProjectIndex | null;
  last_used_at: number;
  links: ProjectLink[];
  missing: boolean;
  name: string;
  origin: Origin;
  root: string;
  /**
   * The lowest score in this project's scope (itself and what it links to); absent while
   * every member is still on its first index.
   */
  scope_health?: number | null;
  selected: boolean;
  state: string;
}
/**
 * One project's record, as stored and as `rtok graph projects --json` prints it.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Capability".
 */
export interface Capability {
  backend: Chosen;
  checked_at: number;
  /**
   * The `backend` value the record was made under; another value makes it stale.
   */
  config: string;
  /**
   * The server was installed and working, then broke. Unlike a server that was never there,
   * this is what the health check raises "backend down" for.
   */
  down?: boolean;
  /**
   * The project's language when it has a marker file.
   */
  language: string | null;
  /**
   * When the health check retries a failed record; absent while the server answers.
   */
  next_probe_at: number | null;
  /**
   * Why the server does not answer; absent while it does.
   */
  reason: string | null;
  /**
   * The language has a language server at all, installed or not.
   */
  server: boolean;
}
/**
 * The 0 to 100 health score with its components, reasons and fixes (T329.19).
 */
export interface Score {
  components: Components;
  level: Level;
  reasons?: Reason[];
  /**
   * Absent while the project's first index runs.
   */
  score: number | null;
}
/**
 * Each component is 0 to 1, rounded to two places.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Components".
 */
export interface Components {
  backend: number;
  freshness: number;
  links: number;
}
/**
 * `graph status` numbers for one project; absent for a missing root, which has nothing
 * readable to count.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ProjectIndex".
 */
export interface ProjectIndex {
  files: number;
  indexed_at: number | null;
  pending: number;
  rows: number;
  watch: string;
}
/**
 * One outgoing link of a project.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "ProjectLink".
 */
export interface ProjectLink {
  kind: LinkKind;
  name: string;
  reason: string | null;
  to: number;
}
/**
 * One session's totals ([`Store::session_totals`], T25.1) — the rendering input of the
 * Sessions page and `rtok agent sessions` (T25.2). Everything below is one query's
 * output, so no renderer can re-derive a number differently (D27): tokens are whole-
 * session sums of `usage`, `last_activity` is the MAX ts over the session's `usage`
 * and `calls` rows (falling back to `started_at` when there are none), and `ended_at`
 * `None` means live.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SessionTotals".
 */
export interface SessionTotals {
  api: string | null;
  cache_create: number;
  cache_read: number;
  ended_at: number | null;
  host: string | null;
  id: string;
  input: number;
  last_activity: number;
  model: string | null;
  output: number;
  project: string | null;
  provider: string | null;
  started_at: number;
}
/**
 * Skills page (T63.1): T61.3 listing joined to T61.1 resident/invocations.
 */
export interface SkillsPage {
  /**
   * Totals: listed, desc bytes ≈ tokens/req (chars/4, research.md §10.2), resident, input share.
   */
  header: string;
  rows: SkillPageRow[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillPageRow".
 */
export interface SkillPageRow {
  body_bytes: number;
  desc_chars: number;
  invocations: number;
  last_invoked: string;
  name: string;
  never: boolean;
  resident: number;
  source: string;
}
/**
 * Tasks page (T441.13.7): the project's `rtok task list --all` as a board, through
 * [`tasks::page`]. Read-only; the slow adapters are read off the tick.
 */
export interface TasksPage {
  board: TaskBoard | null;
  state: TasksState;
  text: string;
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TaskBoard".
 */
export interface TaskBoard {
  /**
   * `disk`, `github` or `gitlab`.
   */
  adapter: string;
  /**
   * Finished tasks left off `tasks`.
   */
  hidden: number;
  /**
   * Every active task, and the newest [`FINISHED_SHOWN`] of each finished status, by id.
   */
  tasks: TaskCard[];
}
/**
 * One task as the board draws it: the fields of `rtok task list --json` except the description,
 * which can be pages long and has no place on a card (`rtok task show` has it).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TaskCard".
 */
export interface TaskCard {
  assignee: string | null;
  /**
   * The tasks this one waits for (`task_dep`), shown as "needs".
   */
  blocked_by: string[];
  conflicts_with: string[];
  external: TaskLink | null;
  id: string;
  parallel: boolean;
  parent: string | null;
  /**
   * `0` is highest. Unlike the CLI's JSON, the default is carried too: a card always has one.
   */
  priority: number;
  status: TaskColumn;
  title: string;
  /**
   * Unix seconds.
   */
  updated_at: number;
}
/**
 * The issue a task lives in on GitHub or GitLab.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TaskLink".
 */
export interface TaskLink {
  adapter: string;
  number: number;
  /**
   * Always `http(s)`: an issue host's reply is untrusted, and the page turns this into a link.
   */
  url: string;
}
/**
 * Overview page: provider usage totals, CTT and the per-turn series.
 */
export interface Overview {
  /**
   * Persistent operator warnings for the whole disabled period (e.g. proxy plain
   * mode). Empty when none; omitted from the wire when empty so the P19 shape stays.
   */
  alerts?: string[];
  cache_create: number;
  cache_read: number;
  /**
   * Σ over sessions of Σ over turns `ctx × turns-after`, where `ctx` is the turn's
   * input-side tokens (`input + cache_create + cache_read`) and `turns-after` counts
   * the session's later turns — the usage-row mirror of `stats`' `tokens × remain`.
   * Output is left out on purpose: it re-enters as a later turn's input or cache,
   * so counting it again would count it twice.
   */
  ctt: number;
  est_after: number;
  est_before: number;
  input: number;
  output: number;
  rows: number;
  /**
   * The Δtok trend Overview and Stats draw: [`SAVINGS_DAYS`] days ending today, oldest
   * first, from [`savings_trend`]. Empty while the first read runs or when the store will
   * not read.
   */
  savings: SavingsDay[];
  /**
   * Per-turn `ctx`, sessions oldest-first and request order within a session, kept
   * to the last [`OVERVIEW_TURNS`]. A `usage` row carries no timestamp, so across
   * sessions this is session order, not wall-clock order.
   */
  turns: number[];
}
/**
 * One day of the Δtok savings trend (T414.13).
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SavingsDay".
 */
export interface SavingsDay {
  /**
   * `YYYY-MM-DD` in `[agents.usage] tz`, so the trend and the Usage page cut the same days.
   */
  day: string;
  /**
   * The same per plugin, only the plugins with rows that day. An `expand` row nets negative,
   * as on the Plugins page.
   */
  plugins: {
    [k: string]: number;
  };
  /**
   * `Measurement` rows stamped that day.
   */
  rows: number;
  /**
   * Σ est_before − est_after; `None` when the day has no rows. No row, no saving claim: an
   * empty day is a gap on the chart, never a zero.
   */
  saved: number | null;
}
/**
 * The Overview page (T15.3): the usage totals plus what the tab draws from them —
 * context-token-turns and the per-turn series behind the sparkline. The totals stay
 * flat under the `usage` key, so the `/ws` frame keeps the shape P19 pinned and the
 * SPA reads on untouched.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Overview".
 */
export interface Overview1 {
  /**
   * Persistent operator warnings for the whole disabled period (e.g. proxy plain
   * mode). Empty when none; omitted from the wire when empty so the P19 shape stays.
   */
  alerts?: string[];
  cache_create: number;
  cache_read: number;
  /**
   * Σ over sessions of Σ over turns `ctx × turns-after`, where `ctx` is the turn's
   * input-side tokens (`input + cache_create + cache_read`) and `turns-after` counts
   * the session's later turns — the usage-row mirror of `stats`' `tokens × remain`.
   * Output is left out on purpose: it re-enters as a later turn's input or cache,
   * so counting it again would count it twice.
   */
  ctt: number;
  est_after: number;
  est_before: number;
  input: number;
  output: number;
  rows: number;
  /**
   * The Δtok trend Overview and Stats draw: [`SAVINGS_DAYS`] days ending today, oldest
   * first, from [`savings_trend`]. Empty while the first read runs or when the store will
   * not read.
   */
  savings: SavingsDay[];
  /**
   * Per-turn `ctx`, sessions oldest-first and request order within a session, kept
   * to the last [`OVERVIEW_TURNS`]. A `usage` row carries no timestamp, so across
   * sessions this is session order, not wall-clock order.
   */
  turns: number[];
}
/**
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Score".
 */
export interface Score1 {
  components: Components;
  level: Level;
  reasons?: Reason[];
  /**
   * Absent while the project's first index runs.
   */
  score: number | null;
}
/**
 * What the user changed since the defaults.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "Selection".
 */
export interface Selection1 {
  /**
   * Entries made the kept copy of their duplicate.
   */
  keep: Ref[];
  /**
   * Entries whose selection is the opposite of their default.
   */
  toggled: Ref[];
}
/**
 * Skills page (T63.1, D23): one row per skill the host lists.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "SkillsPage".
 */
export interface SkillsPage1 {
  /**
   * Totals: listed, desc bytes ≈ tokens/req (chars/4, research.md §10.2), resident, input share.
   */
  header: string;
  rows: SkillPageRow[];
}
/**
 * The Tasks page. `text` is the `rtok task list` screen for the tui and the SPA's status line;
 * `board` is the same rows as data for the SPA. Both come from one list.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "TasksPage".
 */
export interface TasksPage1 {
  board: TaskBoard | null;
  state: TasksState;
  text: string;
}
/**
 * The Usage page: one [`usage::Report`] — the call `rtok agents usage` makes — read once and
 * carried twice. `text` is that command's screen ([`usage::Report::to_text`]) for the tui and
 * the SPA's status line; `report` is the same rows as data for the SPA's tables. Nothing is summed
 * a second time (D27). Both are empty-handed while the first read runs or after it fails:
 * `report` is `None` and `text` says why.
 *
 * This interface was referenced by `WsProtocol`'s JSON-Schema
 * via the `definition` "UsagePage".
 */
export interface UsagePage1 {
  report: Report2 | null;
  text: string;
}
