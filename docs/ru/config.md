---
lang: ru
---

# Конфигурация

Один файл, `~/.rtok/config.toml`, содержит все настройки rtok. **Каждый флаг CLI — это ключ
конфигурации**, поэтому всё, что можно передать в командной строке, можно сделать и постоянным, а
`rtok config show --sources` всегда сообщает, откуда взялось значение.

Статус: T12.1–T12.2 выполнены — каждая таблица ниже является типизированным разделом в `crates/rtok-config/src/lib.rs` (неизвестный
ключ = ошибка), `crates/rtok-config/default.toml` встроен в бинарник и записывается дословно командой `rtok config init`,
`core.inject_budget_tokens` переехал в `plugins.inject.budget_tokens`, а многоуровневость
(user < project < env < flags, на основе `figment`) плюс `config show [--sources] [--json]` и
`config get <key>` уже работают. `config set`/`validate` и тест покрытия флагов появятся в T12.3–T12.4.

## Приоритет

От низшего к высшему. Более поздние уровни переопределяют более ранние по каждому ключу отдельно.

1. **Встроенные значения по умолчанию** — значения из эталонного файла ниже (`crates/rtok-config/default.toml`,
   встроен в бинарник).
2. **Пользовательский файл** — `~/.rtok/config.toml` (каталог переопределяется через `RTOK_HOME`; файл
   переопределяется через `RTOK_CONFIG=<path>` или `--config <path>`).
3. **Файл проекта** — `<git root>/.rtok.toml`, если есть. Схема конфигурации с ограничениями
   доверия ниже; например, разрешены `[plugins.read] default_mode` и `[plugins.inject] budget_tokens`.
4. **Файлы `.env`** — строки `RTOK_*` (те же имена, что и на уровне окружения) из ближайшего
   `.env` в рабочем каталоге или выше, затем `~/.rtok/.env`; разрешённые ключи проекта побеждают.
   Ограничения доверия такие же, как для TOML проекта. Домашний файл считается доверенным;
   отклонённый ключ проекта не скрывает значение из домашнего файла.
   Они только разбираются и никогда не экспортируются: команды, запущенные через `rtok run`, их не наследуют, а другие
   ключи в `.env` проекта игнорируются. О некорректном файле сообщается в stderr, и он пропускается.
5. **Окружение** — `RTOK_<SECTION>_<KEY>` в верхнем snake case, например `RTOK_PROXY_PORT=8791`,
   `RTOK_PLUGINS_CMD_REWRITE=false`, `RTOK_STATS_SINCE=7d`. Списки разделяются запятыми.
   Существующие короткие имена остаются псевдонимами: `RTOK_UPSTREAM`, `RTOK_OPENAI_UPSTREAM`.
6. **Флаги командной строки** — `rtok proxy --port 8791`.

Файлы проекта не могут настраивать разделы `core`, `otel`, `proxy`, `mcp`, `web`
(и устаревший `dashboard`), `log`, `demon`, `agents`, `setup`, `doctor`, `stats`, `report`,
`bench`, `filter`, `worktree`, а также плагины `memory`, `compress`, `graph`, `docs`, `guard`,
`wasm`. Запрещены и ключи `plugins.cmd.shell`, `plugins.cmd.rules`, `plugins.cmd.rules_dir`,
`plugins.read.allow_paths`, `plugins.inject.modes_dir`, `tasks.gitlab.url`.
Они управляют сетевыми адресами, учётными данными, выполнением команд и доступом к файлам.
Ограничения действуют и для устаревших псевдонимов. Предупреждение содержит файл и ключ,
без значения; `config show --sources` отмечает сохранённое безопасное значение как
`ignored (project)`, если его не переопределил более высокий уровень.
Задавайте эти параметры в пользовательском конфиге, `~/.rtok/.env`, окружении процесса
или явно выбранном файле `--config`. Выбор `--config` означает явное доверие файлу,
в том числе внутри проекта.

Правила:

- Позиционные аргументы отдельного вызова (`hook <event>`, `expand <id>`, `run -- <cmd>`,
  `setup <host>`, `memory import <file>`, `--save-baseline <name>`) не являются настройками и не имеют ключа. У всего
  остального ключ есть.
- Флаг `--foo-bar` подкоманды `baz` ↔ ключ `baz.foo_bar`. Настройки плагинов находятся в
  `plugins.<id>.<key>`.
- Неизвестные ключи — ошибка в `rtok config validate` и предупреждение (в stderr, один раз) в остальных местах.
  Хуки никогда не падают из-за проблем с конфигурацией: они пишут в лог и используют значения по умолчанию (fail open).
- Пути принимают `~`. Относительные пути отсчитываются от файла, в котором они указаны.
- Длительности: `30d`, `12h`, `15m`. Размеры: обычные целые числа в байтах или токенах, как указано в имени.

## `rtok config`

| Команда | Что делает |
|---------|------|
| `rtok config show [--sources] [--json]` | итоговая конфигурация после всех уровней; `--sources` помечает каждый ключ как `default / user / project / env / flag` |
| `rtok config init [--force]` | записать эталонный файл (ниже) в `~/.rtok/config.toml`; без `--force` никогда не перезаписывает |
| `rtok config validate [path]` | разобрать, отклонить неизвестные ключи и значения вне диапазона с номерами строк; код выхода 1 при ошибке |
| `rtok config path` | напечатать путь к определённому пользовательскому файлу (и к файлу проекта, если он есть) |
| `rtok config get <key>` | напечатать одно итоговое значение, например `rtok config get proxy.port` |
| `rtok config set <key> <value>` | отредактировать пользовательский файл на месте, сохраняя комментарии (использует `toml_edit`) |

Учётные данные никогда не печатаются: `otel.headers` (он содержит ключи приёма OTLP) и `mcp.token` (bearer-токен
`rtok mcp --http`) после установки показываются как `<redacted>` в `show`, `get`, `set` и `rtok report` — их
источник при этом всё равно показывается. Чтобы увидеть значение, прочитайте сам файл.

## Эталонный файл

Это дословный `crates/rtok-config/default.toml`. Каждое показанное значение — значение по умолчанию; свежий
`rtok config init` записывает ровно это. Логирование — только `[log]` (D26). `core.log_file` / `log_level` / `log_to_db`
из старого файла всё ещё загружаются один раз с предупреждением и сворачиваются в
`[log]`; `rtok config validate` отклоняет эти ключи, потому что их нет в эталонной
схеме.
```toml
# rtok configuration. Every CLI flag has a key here; flags and RTOK_* env vars override.
# Precedence: defaults < this file < <git root>/.rtok.toml < env < flags.
# Docs: docs/config.md. Check: `rtok config validate`. Where a value came from: `rtok config show --sources`.

[core]
enabled     = true                    # false = plain proxy (no business logic); HTTP stays up until process exit
db_path     = "~/.rtok/rtok.db"       # one SQLite file, WAL (decision D8)
archive_dir = "~/.rtok/archive"       # raw payloads for `rtok expand <id>` (decision D4)
session_env = "CLAUDE_SESSION_ID"     # env var consulted for the session id when stdin has none
call_io_inline_bytes = 65536          # MCP/API bodies larger than this go to archive (hooks never archive)
hook_max_input_bytes = 8388608        # rtok hook <event> stdin cap (8 MiB); over it, exits 0 unmodified, no archiving or hashing (T201)
retain_calls_days    = 30             # 0 = keep `calls` forever
retain_hook_bodies_days = 3           # hook stdin bodies cleared after N days, rows kept; 0 = as long as `calls` (T352)
retain_archive_days = 90              # тела архива неактивных сессий удаляются через N дней; 0 = хранить (живой сессии — никогда) (T968)
max_store_mb = 2048                   # db + WAL держатся ниже этого значения сокращением `retain_calls_days` (минимум 7 дней); 0 = без лимита (T968)
max_archive_mb = 1024                 # архив держится ниже этого значения удалением самых старых тел неактивных сессий; 0 = без лимита (T968)

[log]                                 # rtok's own log (D26); `rtok logs` reads it
path      = "~/.rtok/logs/rtok.log"   # rotated siblings live beside it: rtok.log.1 … .5
max_bytes = 1048576                   # rotate past 1 MiB (validate: ≥ 1024)
files     = 5                         # generations kept; older ones are deleted, never archived (validate: ≤ 20)
lines     = 200                       # what `rtok logs` prints when --lines is not given
level     = "info"                    # error | warn | info | debug
to_db     = true                      # also write a `logs` row for `rtok otel`
tspin     = "auto"                    # `rtok logs` through tailspin: auto = terminal and tspin on PATH | always | off (T225.1)

[estimator]                           # chars per token per class, a heuristic (no accuracy figure measured yet); `rtok stats --calibrate` rewrites
code  = 3.5
prose = 4.2
json  = 3.0
cjk   = 1.0

# ── surfaces ────────────────────────────────────────────────────────────────

[hook]                                # rtok hook <event>
host      = "claude"                  # claude | cursor | copilot | devin | cline — payload field mapping (T10.1, T46.3, T87, T94)
max_ms    = 10                        # soft budget; over it, the event is logged as slow
slow_warn_s = 300                     # one slow warning per event per this many seconds, with the count and worst time; 0 = every slow call
fail_open = true                      # any error → `{}` and exit 0; false only for debugging

[agents]                              # the rtok agent registry (T282, D34); see agents-and-worktrees.md
enabled    = true                     # false = hooks skip agent register/touch/end and message push (session bookkeeping is unaffected)
idle       = "30m"                    # `live()`'s window: no `ended_at` and `last_seen` within this of now
push_bytes = 1024                     # framed messages pushed per UserPromptSubmit/PostToolUse; the rest → "and N more" (T288)

[agents.usage]                        # rtok agents usage (T358)
source = "logs"                       # logs = the agents' own session files (Claude Code, Codex, OpenCode, Kilo, Copilot CLI, Gemini CLI); rtok = what passed through rtok; both
hosts  = []                           # [] = every host; else host ids, e.g. ["claude", "codex"]
since  = ""                           # "" = all time; a date (2026-09-01, whole days in tz) or a duration (30d)
until  = ""                           # "" = through today; a date, inclusive
period = "monthly"                    # monthly | daily: the bottom table
by     = "agent"                      # agent | model: what the middle table groups by
tz     = ""                           # IANA zone for day and month boundaries; "" = the system zone

[agents.usage.dirs]                   # where `rtok agents usage` reads each host's own records (T358.3); Claude Code and Codex use [stats] transcripts_dir / codex_dir
opencode = ["~/.local/share/opencode"] # the opencode*.db files in it; an untouched default follows $XDG_DATA_HOME
kilo     = ["~/.local/share/kilo"]     # the kilo*.db files in it; an untouched default follows $XDG_DATA_HOME
copilot  = ["~/.copilot/session-state"] # */events.jsonl; an untouched default follows $COPILOT_HOME
gemini   = ["~/.gemini/tmp"]           # */chats/session-*; an untouched default follows $GEMINI_CLI_HOME
droid    = ["~/.factory/sessions"]     # listed as unsupported when present: Factory does not document the token fields
pi       = ["~/.pi/agent/sessions"]    # */*.jsonl; an untouched default follows $PI_CODING_AGENT_SESSION_DIR, else $PI_CODING_AGENT_DIR/sessions
kimi     = ["~/.kimi-code/sessions"]   # Kimi Code: */*/agents/*/wire.jsonl; an untouched default follows $KIMI_CODE_HOME
grok     = ["~/.grok/sessions"]        # listed as unsupported when present: xAI points at `grok usage`, which rtok does not run; follows $GROK_HOME
zcode    = ["~/.zcode"]                # listed as unsupported when present: ZCode does not document its session records
antigravity = ["~/.gemini/antigravity"] # listed as unsupported when present: Google does not document Antigravity's local data

[agents.junk]                         # rtok agents junk list|clear: age floors, protected paths, your own junk paths
keep_logs_days          = 30          # `logs` entries (the agents' documented log folders) modified within this many days stay; 0-3650
temp_min_age_hours      = 24          # `temp` entries touched within this many hours stay; 0-87600
stale_session_days      = 30          # `--kind sessions` only: sessions not touched for more than this many days are junk, time is the only criterion; 0-3650; `--session-days N` for one run
stale_worktree_days     = 14          # a merged, clean worktree `rtok worktree gc` would remove is junk (`stale-worktrees`, review) once idle this many days; 0-3650
crash_dump_min_age_days = 7           # a crash dump in an `extra` crash-dumps folder older than this many days is safe to clear, a younger one is review; 0-3650
exclude                 = []          # globs (~ = home) never touched, nor any folder holding a match, e.g. ["~/.claude/debug/keep-*"]; a bad glob keeps everything
extra                   = []          # paths you vouch for as junk (D36), e.g. [{ host = "cursor", kind = "cache", path = "~/Library/Application Support/Cursor/CachedData" }]; kind = cache | temp | logs | crash-dumps; host = a host id or rtok

[mcp]                                 # rtok mcp
tools                   = []          # [] = all tools from enabled plugins; else an allow-list; `expand` always stays listed (D4)
max_description_tokens  = 60          # enforced by a test (T4.1)
max_result_chars        = 20000       # above this, head/tail + archive id
http                    = "127.0.0.1:8791"  # `rtok mcp --http` with no address; keep it on loopback
# Через HTTP expand доступны только данные, архивированные этим запущенным сервером; перезапуск сбрасывает доступ.
http_tools              = ["read", "search", "tree"]  # HTTP allow-list, used instead of `tools`; `expand` always stays listed; tools that act as the calling agent (`whoami`, `agent_send`, `worktree_add`, …) never are
token                   = ""          # bearer token for --http; prefer RTOK_MCP_TOKEN; empty = --http refuses to start (T401)
public_url              = ""          # tunnel URL (https://…); its host and origin are the only foreign ones accepted

[proxy]                               # rtok proxy
enabled         = true                # false = plain reverse proxy (bypass compress/bookkeeping); does NOT stop HTTP
bind            = "127.0.0.1"
port            = 8790
mode            = "passthrough"       # passthrough | compress
upstream        = "https://api.anthropic.com"      # RTOK_UPSTREAM; chain behind another proxy for A/B
openai_upstream = "https://api.openai.com"         # RTOK_OPENAI_UPSTREAM (D11)
gemini_upstream = "https://generativelanguage.googleapis.com"  # RTOK_GEMINI_UPSTREAM (T51.3)
timeout_s       = 600                 # upstream request timeout
max_body_bytes  = 8388608             # предел запроса и сохраняемый префикс ответа (8 МиБ); большие ответы продолжают передаваться
max_in_flight   = 8                    # общий лимит, включая режим без lanes; лишние запросы получают 429
include_usage   = true                # OpenAI streaming: add stream_options.include_usage when missing (T11.2)
context_management = false            # Anthropic /v1/messages only: add clear_tool_uses edit + beta header (T51.2, opt-in)
dry_run         = false               # --dry-run: print effective [proxy] settings and exit, don't serve
# TLS: Mozilla webpki roots (`use_preconfigured_tls`). Corporate CAs: SSL_CERT_FILE (PEM, curl). See "TLS and corporate CAs".

[proxy.tools_rewrite]                 # T59.5; off: request bytes stay identical
enabled = false
compact_schema = false                # T686: also first-sentence property descriptions + redundant default/examples inside input_schema; originals archived (Measurement.ref_id)
max_description_tokens = 60           # 0 = no truncate; sentence boundary; estimator Class::Prose
allow = []                            # empty = keep all names not in deny
deny = []                             # drop these names from tools[]; later calls still forward

[proxy.lanes]                         # T385.1; tag each request's lane in the ledger (calls.kind); bytes stay identical
enabled = true                        # false = every request an untagged api_request; x-rtok-lane and /lane/<name>/ forwarded as sent

[proxy.lanes.bulk]                    # синхронные скрипты; lane agent следует глобальным переключателям, batch и files не переписываются никогда
compress            = false           # перезаписи proxy.mode = "compress" (archive, compress, зачистка шума)
toon                = false           # фильтр toon внутри этого прохода; нужен compress
tools_rewrite       = false           # proxy.tools_rewrite
context_management  = false           # proxy.context_management
semantic_cache      = false           # plugins.proxy.semantic_cache, чтение и запись
flex                = false           # OpenAI service_tier = "flex" on this lane; see [proxy.flex]
timeout_s           = 0               # таймаут чтения для этой lane; 0 = proxy.timeout_s, но не меньше 900 при flex = true (гайд OpenAI по Flex берёт 15 мин)
upstream            = ""              # базовый URL для каждого запроса этой lane, любой wire; "" = proxy.upstream / openai_upstream / gemini_upstream
max_in_flight       = 0               # запросов upstream одновременно; 0 = без лимита этой lane (общий лимит всё равно действует)
max_queued          = 8               # при max_in_flight: сколько запросов ждут слот; следующий получает 429 + Retry-After

[proxy.lanes.embeddings]              # те же ключи, что у bulk
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без лимита этой lane; общий лимит всё равно действует
max_queued          = 8

[proxy.lanes.meta]                    # те же ключи, что у bulk (models, подсчёт токенов)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без лимита этой lane; общий лимит всё равно действует
max_queued          = 8

[proxy.lanes.internal]                # те же ключи, что у bulk (собственные вызовы модели rtok)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без лимита этой lane; общий лимит всё равно действует
max_queued          = 8

[proxy.batch]                         # файлы результатов провайдерского Batch
parse_results       = false           # разобрать полученный файл результатов в строку usage на запрос; тело пересылается как есть

[proxy.flex]                          # OpenAI Flex tier; which lanes get it is [proxy.lanes.<lane>] flex
force               = false           # overwrite a service_tier the client sent (off: a client value is never changed)
on_429              = "none"          # Flex has no capacity: none = the 429 goes to the client | backoff = retry on Flex | default = retry once on service_tier "auto" (the client's own tier, if force replaced one)
retries             = 3               # backoff only: retries before giving up; at most 5
backoff_ms          = 1000            # backoff only: delay before the first retry, doubled each time, capped at 30 s; a 429's Retry-After (seconds) can lengthen it, up to that cap

[proxy.routing]                       # model routing; only the internal and bulk lanes, never the agent lane
enabled             = false           # off: every call keeps the model the client sent

[proxy.routing.bulk]                  # one rule per lane; it acts only with from, to and gate all set
from                = []              # request models it applies to: an exact id or a prefix ending in * (empty = off)
to                  = ""              # the model that serves a matching call (empty = off)
provider            = ""              # provider slug recorded for a routed call (empty = the wire's own); pair with [proxy.lanes.<lane>] upstream
gate                = ""              # where the quality bench for this swap is kept (a path, URL or run id); empty = off

[proxy.routing.internal]              # same keys as bulk (rtok's own model calls)
from                = []
to                  = ""
provider            = ""
gate                = ""

[web]                                 # rtok web (same data as rtok tui)
host = "127.0.0.1"                    # --host
port = 3333                           # --port

[tui]                                 # rtok tui (same data as rtok web)
tab       = ""                        # "" = first tab          (--tab <page>)
tick_secs = 2                         # model re-read cadence, same as the web 2 s tick (--tick-secs)

[ui]                                  # rtok's own lines on a terminal; pipes, --json stay plain
emoji = true                          # emoji before status/warn/error lines (RTOK_UI_EMOJI)
color = true                          # colour them; NO_COLOR/CLICOLOR_FORCE apply (RTOK_UI_COLOR)

[stats]                               # rtok stats
since           = "30d"
format          = "table"             # table | json      (--json)
plugin          = ""                  # "" = all         (--plugin <id>)
transcripts_dir = "~/.claude/projects"
codex_dir       = "~/.codex/sessions" # Codex CLI logs → one more `api` row (T49.2); OpenCode and Copilot CLI are read by `rtok agents usage` ([agents.usage.dirs], T358.3); Cursor stores carry no token counts (surveyed 2026-09-17), so they are not read
calibrate_samples = 30                # per class        (--calibrate)
baseline        = ""                  # default name for --compare; "" = none
price           = false               # show per-model USD costs (--price)
# USD per MTok rows for --price (T49.1). Sources, fetched 2026-09-17 (Anthropic claude-fable-5-1,
# claude-opus-5-5 and claude-sonnet-5-5: 2026-10-08):
# Anthropic claude-* rows: https://platform.claude.com/docs/en/about-claude/pricing
# (input / 5m cache write / cache read / output). OpenAI gpt-5 / gpt-5-mini:
# https://platform.openai.com/docs/pricing (short-context input / cached input /
# output; no separate write price, so cache_write = input). Models without a row
# print `-`, never a guess; add dated rows of your own the same way.
[stats.prices."claude-fable-5-1"]
input = 10.0
cache_write = 12.5
cache_read = 0.25
output = 50.0
[stats.prices."claude-opus-5-5"]
input = 4.0
cache_write = 5.0
cache_read = 0.2
output = 20.0
[stats.prices."claude-sonnet-5-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.2
output = 10.0
[stats.prices."claude-haiku-4-5"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."gpt-5"]
input = 1.25
cache_write = 1.25
cache_read = 0.125
output = 10.0
[stats.prices."gpt-5-mini"]
input = 0.25
cache_write = 0.25
cache_read = 0.025
output = 2.0
# Уровни Batch и Flex, получено 2026-10-08: ключ — `<model>@batch` или `<model>@flex`, а `--price`
# считает usage полосы Batch по строке `@batch`. Anthropic Batch — скидка 50 % на ввод и вывод,
# множители кеша накладываются сверху (страница цен выше; уровня Flex нет). У OpenAI Batch и Flex
# указаны одинаковые ставки (https://developers.openai.com/api/docs/pricing).
[stats.prices."claude-fable-5-1@batch"]
input = 5.0
cache_write = 6.25
cache_read = 0.125
output = 25.0
[stats.prices."claude-opus-5-5@batch"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.05
output = 5.0
[stats.prices."claude-sonnet-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."claude-haiku-4-5@batch"]
input = 0.5
cache_write = 0.625
cache_read = 0.05
output = 2.5
[stats.prices."gpt-5@batch"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5@flex"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5-mini@batch"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0
[stats.prices."gpt-5-mini@flex"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0

[report]                              # rtok report (D24: renders the operator model, computes nothing)
format = "md"                         # md; html (T22.2), pdf (T22.3), --ai (T22.4)
out    = ""                           # "" = stdout     (--out <path>)
since  = "30d"                        # how far back the report reads
ai     = false                        # model-shaped rendering instead of --format (--ai)
budget_tokens = 8000                  # --ai drops whole sections past this (T22.4)

[bench]                               # rtok bench
tasks    = "bench/tasks.toml"
runs     = 3
dry_run  = false
timeout_s = 900                       # per task run
suite    = ""                         # "" = T9.1 six tasks; "graph" = T68.9 with/without MCP; "p28" = T385.9.3 compressor vs lossless
[bench.configs]                       # name = settings file passed to `claude --settings`
a = "bench/configs/legacy.json"
b = "bench/configs/rtok.json"

[doctor]                              # rtok doctor
settings_path   = "~/.claude/settings.json"
claude_json     = "~/.claude.json"
mcp_json        = ".mcp.json"
probe_timeout_ms = 500                # per proxy hop /health probe
mcp_timeout_ms  = 15000               # per MCP server tools/list (uvx/npx servers start slowly)
instruction_warn_tokens = 1000        # --instructions: flag files above this
instructions    = false               # run the instruction audit by default (--instructions)

[setup]                               # rtok agents install / remove <host>
dry_run      = false
yes          = false                  # required by --replace
backup       = true                   # <name>.bak-<ts> beside each file, before setup and remove touch it
backup_files = 5                      # .bak-* generations kept per file; older ones are deleted (0 = keep all)
hook_timeout_s = 5                    # timeout written into each hook entry
modes        = []                     # e.g. ["terse", "yagni"]   (--mode)
mcp          = true                   # also register the MCP server   (--mcp)
proxy        = false                  # also set the base URL          (--proxy)
[setup.paths]                         # host = "path": where a host app is when rtok cannot find it (T966; `agents install|update|info` asks and offers to save)
# e.g. claude = "~/.local/bin/claude"   # a CLI binary, or a desktop app (`/Applications/Claude.app`, `claude.exe`)
[setup.claude]
settings_path = "~/.claude/settings.json"
[setup.cursor]
hooks_path    = "~/.cursor/hooks.json"  # today: beforeShellExecution only (T10.11 wires PostToolUse)
[setup.codex]
config_path   = "~/.codex/config.toml"
[setup.opencode]
config_path   = "~/.config/opencode/opencode.json"
[setup.kilo]
config_path   = "~/.config/kilo/kilo.json"      # kilo.jsonc is merged by Kilo, never rewritten
[setup.pi]
extensions_path = "~/.pi/agent/extensions"
tools           = false                     # pi.registerTool for read/search/graph/memory (T70.3)
[setup.omp]
extensions_path = "~/.omp/agent/extensions" # oh my pi: plugins/pi is linked here (T92)
mcp_path        = "~/.omp/agent/mcp.json"
[setup.zcode]
config_path   = "~/.zcode/cli/config.json"
[setup.kimi]
config_path   = "~/.kimi-code/config.toml"  # mcp.json is read beside it
[setup.copilot]
dir           = "~/.copilot"                # mcp-config.json, hooks/rtok.json
[setup.commandcode]
dir           = "~/.commandcode"            # settings.json (hooks key), mcp.json
[setup.aider]
config_path   = "~/.aider.conf.yml"         # openai-api-base → rtok proxy (--proxy)
[setup.windsurf]
config_path   = "~/.codeium/windsurf/mcp_config.json"
[setup.zed]
config_path   = "~/.config/zed/settings.json"
[setup.cline]
hooks_path = "~/Documents/Cline/Hooks"
mcp_path = "~/.cline/data/settings/cline_mcp_settings.json" # CLI; the extension uses the VS Code globalStorage settings file
[setup.gemini]
dir = "~/.gemini" # settings.json (hooks, mcpServers)
[setup.codewhale]
dir = "~/.codewhale" # config.toml ([[hooks.hooks]]), mcp.json (mcpServers)
[setup.mimo]
config_path   = "~/.config/mimocode/mimocode.json" # mcp (OpenCode-fork shape)
[setup.antigravity]
plugins_path     = "~/.gemini/config/plugins"          # Antigravity 2.0 / IDE: plugins/antigravity linked here
cli_plugins_path = "~/.gemini/antigravity-cli/plugins" # agy plugin install stages here; read only
[setup.devin]
config_path   = "~/.config/devin/config.json"  # mcp_config.json is read beside it; Windows: %APPDATA%\devin\
[setup.roo]
mcp_path      = ""                               # empty: <Code user dir>/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json
[setup.junie]
mcp_path      = ""                               # empty: ~/.junie/mcp/mcp.json (Junie CLI user MCP file)
[setup.qwen]
dir           = "~/.qwen"                    # settings.json (hooks, mcpServers); QWEN_HOME moves the directory
[setup.codebuddy]
dir           = "~/.codebuddy"               # CodeBuddy Code: settings.json (hooks), .mcp.json, plugins/; CODEBUDDY_CONFIG_DIR moves it
workbuddy_dir = "~/.workbuddy"               # WorkBuddy Desktop: the same files (embedded CodeBuddy Code; unverified)
[setup.goose]
config_path   = "~/.config/goose/config.yaml"  # Goose CLI and Desktop: extensions.rtok; Windows: %APPDATA%/Block/goose/config/config.yaml; GOOSE_PATH_ROOT: <root>/config/config.yaml
[setup.droid]
mcp_path      = "~/.factory/mcp.json"        # Factory Droid: пользовательский файл MCP-серверов (mcpServers)
[setup.kiro]
mcp_path      = "~/.kiro/settings/mcp.json"      # Kiro IDE and CLI: user-level MCP servers (mcpServers)
[setup.amp]
settings_path = "~/.config/amp/settings.json"    # Amp: user settings; amp.mcpServers holds MCP servers (settings.jsonc if it is the only one)
[setup.warp]
mcp_path      = "~/.warp/.mcp.json"          # Warp: пользовательский файл MCP-серверов (mcpServers)
[setup.continue]
config_path   = "~/.continue/config.yaml"        # Continue (VS Code, JetBrains, cn): mcpServers item rtok; Windows: %USERPROFILE%/.continue/; CONTINUE_GLOBAL_DIR moves it
[setup.crush]
config_path   = "~/.config/crush/crush.json" # Crush: global JSON config (mcp.rtok); CRUSH_GLOBAL_CONFIG or XDG_CONFIG_HOME move it
[setup.openhands]
mcp_path      = "~/.openhands/mcp.json"     # OpenHands: пользовательский файл MCP-серверов (mcpServers)
[setup.augment]
settings_path = "~/.augment/settings.json"   # Auggie CLI: пользовательские настройки, mcpServers
[setup.reasonix]
dir           = "~/.reasonix"                # Reasonix: config.toml (MCP-серверы в [[plugins]]); REASONIX_HOME переносит каталог (Windows: %APPDATA%/reasonix)

[expand]                              # rtok expand <id>
max_lines = 0                         # 0 = unlimited   (--lines a-b is per call)
max_rate  = 0.05                      # re-read ceiling; above it the report flags lossy compression (T22.5)

[filter]                              # rtok filter --stdin (T10.2)
cmd = ""                              # command family hint when the caller knows it (--cmd)

[worktree]                            # rtok worktree add | claim | remove | list | gc
enabled = true                        # false: every `rtok worktree` command (list too) says worktrees are not enabled, MCP lists no worktree_* tool, and Claude's WorktreeCreate/WorktreeRemove hooks do what Claude does without rtok
root = ".rtok/worktrees"              # where `rtok worktree add` creates worktrees, as <root>/<repo>-<task>; relative: under the main checkout; `~` expands
seed = true                           # fill a new worktree's seed_dirs from the sibling checkout that built them last: copy-on-write clones (APFS, btrfs, XFS, bcachefs, ReFS, Dev Drive), a plain copy elsewhere; `rtok worktree add --no-seed` skips it once
seed_dirs = ["target", "node_modules"] # build caches to seed, relative to the checkout; a bare name also matches one level down (web/node_modules)
min_free_mib = 2048                    # `rtok worktree add` refuses when the target volume has less free space than this, in MiB (a full disk leaves a half-made worktree); 0 turns the check off

[tasks]                               # адаптеры задач (T441); обычно задаются для проекта в .rtok.toml
adapter = "disk"                      # disk | github | gitlab
prefix = ""                           # префикс id задач, 1–8 ASCII-букв (R → R12, R2.1); пусто: первая буква имени проекта

[tasks.disk]
dir = "tasks"                         # где лежат Markdown-задачи, относительно корня проекта
layout = "auto"                       # files: "R12 - slug.md", выполненные в <dir>/done; folders: "<id>/task.md", закрыта файлом "<id>/done.md" (plan/); auto: folders, если <dir> уже так устроен

[tasks.github]
repo = ""                             # owner/name; пусто: remote origin
project = 0                           # номер Projects v2 владельца репо: issues попадают в него, поле Status следует за задачей; 0 = только issues

[tasks.gitlab]                        # метки status::in-progress | status::done | status::wont-do; подзадача связана с родителем (relates_to)
url = "https://gitlab.com"            # https базовый URL; задайте для своего инстанса; токен: GITLAB_TOKEN, GITLAB_ACCESS_TOKEN, GL_TOKEN, иначе glab
project = ""                          # group/name или числовой id; пусто: remote origin

[tasks.export.github]                 # `rtok task export github`: односторонне отразить задачи с диска в GitHub Issues
repo = ""                             # owner/name; пусто: [tasks.github] repo, иначе remote origin

[tasks.export.linear]                 # `rtok task export linear`: односторонне отразить задачи с диска в Linear; токен в LINEAR_API_KEY
team = ""                             # короткий ключ команды, например "ENG"; пусто: экспорт не запускается

[otel]                                # OpenTelemetry export (D19); off until endpoint resolves
endpoint      = ""                    # OTLP/HTTP base URL, e.g. "http://localhost:4318"; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                    # "k=v,k2=v2", e.g. "signoz-ingestion-key=…"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"                # resource service.name
content       = false                 # явно включите, чтобы отправлять тела сообщений, аргументы и результаты в спанах
content_bytes = 65536                 # per attribute; beyond it rtok.archive.id → `rtok expand <id>`
flush_secs    = 5                     # proxy / mcp flush interval, and the POST timeout

# ── plugins ─────────────────────────────────────────────────────────────────

[plugins.measure]
enabled = true

[plugins.cmd]
enabled  = true
rewrite  = true                       # PreToolUse(Bash) → `rtok run -- …`
shell    = ""                         # "" = $SHELL
rules    = "~/.rtok/rules.toml"       # extra filter rules; missing → built-in rules/default.toml
rules_dir = "~/.rtok/rules.d"         # drop-ins: every *.toml merges after rules in name order (T50.2)
trailer  = true                       # print `[rtok <id8> · N lines]` after a cut; false = `--no-ref`
trailer_min_lines = 40                # add `[rtok <id8> · N lines]` above this
fail_tail_lines   = 80                # non-zero exit → last N lines verbatim
never_wrap = ["rtok", "sudo"]         # first-word deny list; heredocs, `&`, -i are always skipped

[plugins.read]
enabled          = true
default_mode     = "full"             # full | lines | map | signatures
max_chars        = 20000              # above this, head/tail + archive id
native_max_bytes = 32768              # PreToolUse(Read) deny threshold; never below this
range_max_lines = 300              # T383: a native Read with limit 1..=N passes the hook; 0 = unranged only
advice           = true               # false = never deny native Read
allow_paths      = []                 # extra roots outside cwd
search_max       = 50
search_max_bytes = 1048576             # search skips files larger than this (T55.5)
tree_depth       = 2
delta            = true               # T58.1: changed re-read → unified diff vs last archive (7.3 % of Read bytes, 2026-09-18, `rtok stats --since 90d`)
delta_max_ratio  = 0.6                # full file when the diff is not below this fraction

[plugins.json_tree]
enabled = false                     # fold repeated nested JSON in old tool results; off until a measurement shows a saving

[plugins.archive]
enabled    = true
keep_turns = 4                        # never touch the last N turns
min_tokens = 1500                     # only rewrite tool results above this (estimated)
head_lines = 8
tail_lines = 4
tiers      = false                    # opt-in tiered loading (default off); OpenViking L0/L1/L2 behaviour spec is AGPL-3.0 — rtok does not vendor, link, or subprocess it (D6); gates native impl in T33.2
live_blobs = false                    # shrink nested JSON dumps + data: blobs in user blocks, never results/system/tools/last-2-turns (T51.1, opt-in)
skills     = false                    # archive skill bodies outside keep_turns (T61.2, opt-in)

[plugins.proxy]
enabled = true                        # the proxy plugin (usage capture); the server itself is [proxy]

[plugins.inject]
enabled       = true
budget_tokens = 800                   # per turn, all injections together (decision D5)
modes_dir     = "~/.rtok/modes"
modes         = []                    # same as [setup].modes; setup writes here

[plugins.guard]
enabled      = true
window_turns = 8
deny_grep_glob = false           # opt-in: deny native Grep/Glob, point at MCP search/tree (T50.4)
grep_symbol = false              # opt-in: a Grep for one identifier (`foo`, `fn foo`, `class Foo`, `\bfoo\(`) is denied with its 1-5 indexed definitions + reference count; index lookup only (T369)
skills = false                   # opt-in (Claude Code): deny a Skill whose SKILL.md exceeds skill_max_bytes with its map + `expand <id>` (T62.1)
skill_max_bytes = 8192           # bodies at or under this load whole; so does any skill with allowed-tools / model / context / agent in its frontmatter

[plugins.memory]
enabled        = true
recall_titles  = 5                    # SessionStart: last N titles + ids
recall_tokens  = 200
prompt_recall  = 5                    # UserPromptSubmit: 0 = off; N = ranked titles per turn (T69.5)
checkpoint_tokens = 400               # PreCompact → SessionStart(compact)
search_limit   = 5
sync_tokens    = 300                  # rtok memory sync: CLAUDE.md / AGENTS.md block (T69.6)
startup_recall = true                 # SessionStart(startup) restores newest session:* note (T71.2)
handoff        = true                 # T59.6 sub-agent digest MCP tool
spawn_brief        = true             # T130: SubagentStart pointer digest
spawn_brief_tokens = 300              # T130: token budget for the spawn brief
admit              = true             # T671: mem_save refuses a too-short or duplicate note
admit_min_chars    = 5                # T671: shortest body the gate admits
admit_similarity   = 85               # T671: percent trigram overlap that counts as a duplicate
forget_after_days = 90                # T669: `memory forget` retires notes unread this long; 0 = needs --days

[plugins.memory.embed]
enabled    = false                    # P29: FTS5-only when false; vector search is opt-in
provider   = "local"                  # "local" = офлайн-хеш-векторы | "http" = OpenAI-совместимый API эмбеддингов
model      = "all-MiniLM-L6-v2"       # "http": имя модели на сервере
dimensions = 384                      # "http": должно совпадать с длиной вектора сервера
hybrid     = true                     # когда включено: RRF(fts5, knn) для mem_search и prompt_recall; хук читает только сохранённые векторы; false = только knn
url         = ""                      # "http": базовый URL, напр. "http://localhost:11434/v1" (Ollama) или "https://api.openai.com/v1"; текст заметок уходит туда
api_key_env = ""                      # "http": имя переменной окружения с ключом; пусто = без заголовка Authorization
max_chars   = 8000                    # "http": более длинный текст не отправляется (работает обычный поиск по заметкам)

[plugins.memory.extract]
enabled         = false                 # rtok memory extract: off by default, a model call can cost money
command         = []                    # program reading the transcript on stdin, one {"title","body"} JSON line out per note
max_notes       = 5                     # notes kept per run
max_input_bytes = 60000                 # transcript bytes sent to the program; the end of the session is kept

[plugins.graph]
enabled    = true
max_tokens = 2000                     # per response; beyond it: head + "N more, expand <id>"
impact_tokens = 1500                  # impact: files grouped and ranked up to this budget, then "+K files, M refs not shown"; --all or 0 = every row
map_tokens = 0                        # SessionStart repo map cap (D5 share next to memory.recall_tokens); 0 = off until a P7 A/B passes
map_rank   = "pagerank"               # SessionStart map order: refs = references per name; pagerank = files by personalized PageRank, personalized by recently edited files and the last checkpoint after a compact
body_lines = 40                       # symbol(): source lines shown per definition
auto_index = true                     # true = every call walks the tree; false = index once, then `rtok graph index` or the watcher (a hook-staled file reads as missing until then)
auto_add_projects = true               # T329.6: register a directory in the project registry when a hooked session starts there, a worktree is made or adopted through `rtok worktree` (named by its branch), or a graph MCP call runs there; false = the registry changes only through the page and the CLI
backend    = "tags"                   # tags | lsp | auto | text: tags = tree-sitter index (default); lsp = language server from PATH, tags when it cannot answer; auto = per project and language, server first, tags second, text search when no grammar parses the project; text = in-process text search only (no call graph, no dead)
lsp_timeout_ms = 40000                # how long one language-server wait (starting, still indexing) may take before the request falls back to tags
backend_by_language = {}              # backend for one language, e.g. { go = "tags", rust = "lsp" }
watch      = "off"                    # off | notify: background re-index inside `rtok mcp` (P8d)
auto_link_references = true           # T329.8: follow references in manifests (Cargo path, npm file:/link:, go replace, Python path, submodules) into other directories, register and auto-link them
reference_depth = 3                   # T329.8: reference levels followed from the project (A -> B is 1); reaching it is shown and logged
max_auto_projects = 20                # T329.8: most projects references may add to the registry; reaching it is shown and logged
alerts = true                         # T329.17: alert when a project in the scope is missing, unreachable, has its backend down or a broken manifest link; shown by rtok doctor, graph projects and graph answers
health_check_interval_s = 60          # T329.17: seconds between health checks in rtok mcp and rtok web (an alert needs two in a row); 0 = off
live_heat_window_s = 300            # T329.34: секунд, сколько узел остаётся «тёплым» на живом холсте графа после вызова (rtok web, rtok tui)
live_max_events_per_s = 50          # T329.34: сколько событий вызовов живая панель показывает в секунду; счётчики считают каждый вызов
live_feed_rows = 200                # T329.34: сколько завершённых вызовов хранит живая лента (1-1000)

[plugins.toon]
enabled  = true
min_rows = 5

[plugins.compress]
enabled = true                        # extractive summaries of archived tool output; runs only in proxy.mode = "compress"

[plugins.compress.llm]
enabled           = false             # P28: model summaries of archived output via the internal lane; needs proxy.mode = "compress"
url               = ""                # where the compressor posts; empty = this proxy (proxy.bind:proxy.port)
model             = "claude-haiku-4-5"
max_input_bytes   = 65536             # larger archived results keep the extractive view
max_output_tokens = 512
max_spans         = 64                # more must-keep spans than this keep the extractive view
max_in_flight     = 2                 # compressor calls at once
timeout_s         = 60
code              = false             # also summarise diffs and source; off = logs and prose only

[plugins.docs]
enabled        = false                # local rustdoc from Cargo.lock + cached docs.rs JSON (T455)
query_limit    = 5                    # потолок попаданий docs_query
snippet_chars  = 400                  # символов на FTS-фрагмент
max_tokens     = 800                  # потолок оценки всего ответа; лишние попадания отбрасываются

[plugins.wasm]
enabled = false                      # off by default; no .wasm loaded until T32.2 host + `wasm-host` feature
dir     = "~/.rtok/plugins"          # scan one level for *.wasm; D6 — this repo never vendors third-party plugins
```

### `[plugins.proxy.semantic_cache]` — явно включаемый кеш ответов (P31)

По умолчанию выключен до прохождения Gate P31 (ноль ложных попаданий на наборе P9). Когда он включён (T31.2), прокси может
отдать предыдущий ответ, если нормализованный промпт достаточно похож; ложное попадание означает неверный ответ, поэтому
он остаётся включаемым явно. Окружение: `RTOK_PLUGINS_PROXY_SEMANTIC_CACHE_ENABLED=true`.

| Ключ | По умолчанию | Значение |
|-----|---------|---------|
| `enabled` | `false` | Главный переключатель; при выключенном байты прокси остаются идентичными |
| `threshold` | `0.99` | Нижний порог косинусного сходства для семантического уровня |
| `ttl_s` | `300` | TTL записи в секундах |
| `max_messages` | `1` | Пропускать кеш, когда длина `messages` превышает это значение |
| `require_empty_tools` | `true` | Не кешировать ходы с непустым `tools[]` |
| `embed_backend` | `"hash"` | `"hash"` = только прямой уровень до появления эмбеддингов P29 |
| `cache_by_model` | `true` | Разделять записи кеша по модели |
| `cache_by_provider` | `true` | Разделять записи кеша по провайдеру |

```toml
[plugins.proxy.semantic_cache]
enabled = false
threshold = 0.99
ttl_s = 300
max_messages = 1
require_empty_tools = true
embed_backend = "hash"
cache_by_model = true
cache_by_provider = true
```

### `[proxy.lanes]`

Каждый проксируемый запрос относится к одной из lane, и lane записывается в журнал
(`calls.kind`). Сначала решает путь: Batch (`/v1/messages/batches`, `/v1/batches`,
Gemini `:batchGenerateContent`), `files`, `embeddings` и `meta` (`/v1/models`,
`count_tokens`) задают собственную lane. Синхронный вызов чата — это ход `agent`, если вызывающая
сторона не указала иное заголовком `x-rtok-lane: bulk|internal` или префиксом пути `/lane/<name>/`
(`/lane/bulk/v1/messages`). Оба убираются перед отправкой запроса upstream. Эвристик нет:
запрос без пометки — это `agent`, lane, в которой был каждый запрос до появления lanes.

| Ключ | Тип | По умолчанию | Значение |
|-----|------|--------------------|---------|
| `enabled` | bool | `true` | `false` записывает каждый запрос как обычный `api_request` и пересылает заголовок и префикс без изменений |

Lane agent сохраняет `calls.kind = api_request`; остальные записывают `api_request:<lane>`
(`api_request:bulk`, `api_request:batch`, ...). Байты запроса из-за lane не меняются.

У lane `agent` таблицы нет: для неё каждый глобальный переключатель решает ровно так, как до
появления lanes (стабильность prompt-кэша). У `batch` и `files` её тоже нет: их тела
(Batch JSONL, загрузки) всегда пересылаются без изменений, а правка `stream_options` для них
тоже пропускается. `bulk`, `embeddings`, `meta` и `internal` читают каждая свою таблицу
`[proxy.lanes.<lane>]`, и все переключатели в ней по умолчанию выключены, поэтому эти lanes
пересылаются байт в байт, пока вы не включите одну из них. Переключатель lane лишь сужает
глобальный: перезапись выполняется на lane, когда включены глобальный переключатель *и*
переключатель lane.

| Ключ | Какой глобальный переключатель сужает | По умолчанию | Значение |
|-----|--------------------------|---------|---------|
| `compress` | `proxy.mode = "compress"` | `false` | archive, compress и зачистка шума терминала |
| `toon` | `plugins.toon.enabled` | `false` | фильтр `toon` внутри этого прохода; нужен `compress` |
| `tools_rewrite` | `proxy.tools_rewrite.enabled` | `false` | переписывание описаний в `tools[]` |
| `context_management` | `proxy.context_management` | `false` | серверное редактирование контекста Anthropic |
| `semantic_cache` | `plugins.proxy.semantic_cache.enabled` | `false` | чтение и запись кэша |
| `flex` | нет | `false` | OpenAI `service_tier = "flex"` для вызовов chat и responses, см. [`[proxy.flex]`](#proxyflex) |
| `timeout_s` | `proxy.timeout_s` | `0` | таймаут чтения этой lane в секундах; `0` = `proxy.timeout_s`, но не меньше 900 при `flex = true` (руководство OpenAI по Flex поднимает таймаут SDK до 15 минут: запросы Flex чаще упираются в таймаут) |
| `upstream` | `proxy.upstream`, `openai_upstream`, `gemini_upstream` | `""` | базовый URL для каждого запроса этой lane, какой бы ни был wire (шлюз или локальный сервер, который их понимает); `""` = собственный upstream wire |
| `max_in_flight` | нет | `0` | сколько запросов этой lane одновременно у upstream, считая до конца потока ответа; `0` = без лимита этой lane, общий лимит всё равно действует |
| `max_queued` | нет | `8` | при `max_in_flight`: сколько запросов ждут слот; следующий получает `429` с `Retry-After: 1` и до upstream не доходит |

У каждой lane свои слоты и своя очередь; `max_in_flight = 0` отключает только лимит этой lane.
Общий лимит применяется ко всем запросам, включая `agent`, plain mode и отключённые lanes;
слот удерживается до конца потока ответа. Запрос, которому отказал любой переполненный лимит,
получает `rate_limit_error` в формате Anthropic, на который SDK провайдеров отвечают паузой и
повтором; строка в `calls` не пишется (запрос не дошёл до upstream). Переполнение очереди lane
также пишет строку `warn`; общий лимит не пишет отдельный лог на каждый отказ. При выключенном
прокси (`proxy.enabled`, `core.enabled` или `plugins.proxy.enabled` равны false) лимиты и
очереди lane обходятся, но общий предел ресурсов сохраняется.
У `batch` и `files` нет `upstream`: создание, опрос и результаты Batch-задачи всегда идут к
провайдеру, которому она принадлежит.

У lane `agent` нет переключателя `flex`: живой ход не меняет уровень, пока клиент сам
его не попросит.

### `[proxy.batch]`

Наблюдение за провайдерским Batch. Сами вызовы Batch (создание, опрос, список, отмена, результаты)
уже помечены `api_request:batch` через `[proxy.lanes]`, а их тела никогда не переписываются.

| Ключ | Тип | По умолчанию | Значение |
|-----|------|--------------|---------|
| `parse_results` | bool | `false` | После пересылки файла результатов записывает по одной строке `usage` на успешный запрос: Anthropic `GET /v1/messages/batches/{id}/results` и OpenAI `GET /v1/files/{id}/content`, если его строки — результаты Batch (такой вызов получает метку `api_request:batch`). Строки с ошибкой, истёкшие и повреждённые пропускаются. Нужен `[proxy.lanes] enabled`; байты ответа не меняются. |

```toml
[proxy.batch]
parse_results = false
```

### `[proxy.flex]`

Обработка Flex в OpenAI тарифицирует вызов chat или responses по ставкам Batch в обмен на задержку
([OpenAI: Flex processing](https://developers.openai.com/api/docs/guides/flex-processing), проверено
2026-10-08). При `flex = true` в lane (`bulk`, `internal`, `embeddings` или `meta`; у `agent`, `batch`
и `files` его нет) rtok задаёт `service_tier = "flex"` в вызовах этой lane к `/v1/chat/completions`
и `/v1/responses`. У Anthropic нет уровня Flex, его протокол не затрагивается никогда.

Переданный клиентом `service_tier` (`auto`, `default`, `priority`, `flex`) не перезаписывается, пока не
задано `force = true`. Если поля нет, rtok добавляет его в начало объекта, а все остальные байты
пересылает так, как их прислал клиент.

Когда у Flex нет мощности, OpenAI отвечает `429 Resource Unavailable` и ничего не списывает. Код ошибки
в руководстве не назван, поэтому rtok считает так любой `429` на запрос, которому сам задал Flex. Запрос,
у которого Flex задал клиент, не повторяется: этот `429` обрабатывает клиент.

| Ключ | Тип | По умолчанию | Значение |
|-----|------|---------|---------|
| `force` | bool | `false` | Перезаписывать переданный клиентом `service_tier` значением `flex` |
| `on_429` | string | `"none"` | `none` отдаёт `429` клиенту; `backoff` повторяет на Flex с удваиваемыми паузами; `default` повторяет один раз с `service_tier = "auto"` (стандартная обработка, дороже) |
| `retries` | int | `3` | только `backoff`: повторов до того, как последний `429` уйдёт клиенту; не более `5` |
| `backoff_ms` | int | `1000` | только `backoff`: пауза до первого повтора, удваивается, максимум 30 с |

Пока rtok повторяет запрос, клиент ждёт и видит только итоговый ответ. Повтор `default` возвращает
`service_tier`, который прислал клиент, если `force` его заменил, и ставит `auto` только когда клиент
его не прислал. `Retry-After` на `429` (только секунды; дата и прочее игнорируются) удлиняет паузу до
большего из него и задержки backoff, но не больше 30 с. Если он просит больше, ждать не станут: `backoff`
отдаёт `429` клиенту, `default` сразу повторяет на запасном уровне. `408` не повторяется и доходит до
клиента без изменений. Повторы пишутся в лог на уровне `warn`.

```toml
[proxy.lanes.bulk]
flex = true

[proxy.flex]
force = false
on_429 = "backoff"
```

### `[proxy.routing]`

Маршрутизация моделей (D9) переводит вызов на более дешёвую модель, и только там, где риск для
качества минимален: собственные вызовы rtok в lane `internal` и lane `bulk`, включённый явно
(`docs/research/optimization.md` §4.2). Lane `agent` не маршрутизируется никогда: живой ход идёт
на модели, которую выбрал оператор. `batch` и `files` тоже никогда не переписываются.
Выключено по умолчанию, правил в поставке нет.

| Ключ | Тип | По умолчанию | Значение |
|-----|------|---------|---------|
| `enabled` | bool | `false` | Главный переключатель; выключен — каждый вызов идёт на модели, которую прислал клиент |

В `[proxy.routing.bulk]` и `[proxy.routing.internal]` лежит по одному правилу. Правило работает,
только когда заданы все четыре ключа ниже; недописанное правило ничего не делает.

| Ключ | Тип | По умолчанию | Значение |
|-----|------|---------|---------|
| `from` | список строк | `[]` | Модели запроса, к которым применяется правило: точный id или id с `*` на конце как префикс (`claude-opus-*`). Модель вне списка уходит как есть |
| `to` | string | `""` | Модель, которая обслужит подходящий вызов |
| `provider` | string | `""` | Slug провайдера, записываемый для маршрутизированного вызова, если `to` живёт не у провайдера wire; `""` = провайдер wire |
| `gate` | string | `""` | Где хранится бенч качества для этой замены (путь, URL или id прогона) |

`gate` — это контроль качества I-101. Достаточно ли хорошо отвечает дешёвая модель — результат
бенча, а не то, что прокси может увидеть, поэтому rtok требует от оператора сначала прогнать бенч
на реальной нагрузке lane и назвать его здесь; сам он его не читает и не оценивает. Без `gate`
правило выключено.

Заменяется только JSON-поле `model` (тело сериализуется заново); Gemini указывает модель в пути,
который маршрутизация не переписывает, поэтому вызов Gemini не маршрутизируется никогда. Замена
происходит до записи вызова, так что `calls` и `usage` называют модель, которая обслужила вызов
(и `provider`, если он задан), `rtok stats --price` оценивает lane по цене этой модели, а строка
журнала вызова хранит модель, которую просил клиент. Каждый маршрутизированный вызов получает
также строку измерения `model_route`; экономии токенов она не заявляет, потому что экономия — это
доллары: сравнивайте `cost by lane` из `rtok stats --price` на одной и той же нагрузке с выключенной
и включённой маршрутизацией. Правило между провайдерами сочетайте с `[proxy.lanes.<lane>] upstream`
для шлюза, который обслуживает `to`; rtok не переводит между wire, поэтому `to` должна быть
моделью, которую принимает API этого wire.

```toml
[proxy.routing]
enabled = true

[proxy.routing.bulk]
from = ["claude-sonnet-*"]
to = "claude-haiku-4-5"
gate = "bench/bulk-haiku-2026-10.md"
```

### Цены для stats (`[stats.prices]`)

`rtok stats --price` оценивает строки `usage` прокси в USD: каждую составляющую — по её
строке `$` за MTok, `cost` — их сумма, `saved` — сколько сэкономило чтение из кеша по сравнению
с ценой некешированного ввода; это единственная экономия, которую можно вычислить только по строкам `usage`.
Для модели без строки в обоих долларовых столбцах печатается `-` (количество её токенов
всё равно печатается); добавьте собственную датированную строку, а не угадывайте. Поставляемые
строки взяты со страниц цен провайдеров 2026-09-17 (источники в
`crates/rtok-config/default.toml`); перепроверьте их, если ваш счёт не сходится. `stats.price`
включает отображение `--price` по умолчанию (`RTOK_STATS_PRICE=true` тоже работает).

Строка с ключом `<model>@batch` (или `<model>@flex`) оценивает модель на уровне Batch (или Flex). `--price`
выводит usage вызовов полосы Batch под `<model>@batch` и считает его по этой строке; без неё печатает `-`,
а не берёт стандартную ставку. Вызовы, которые провайдер сообщил как обслуженные на Flex (`service_tier` в ответе, записывается в
`calls.service_tier`), выводятся под `<model>@flex` и считаются так же. `rtok stats` и `rtok report` добавляют таблицу
`lane/tier` со счётчиками кэша, когда трафик шёл вне полосы agent или на уровне, отличном от `standard` / `default`;
вызов, в ответе которого уровень не назван, печатается с `-`.

Когда трафик пошёл вне полосы agent, `--price` печатает ещё и `cost by lane` (в JSON — `cost.lanes`):
доллары каждой lane, где каждый вызов оценён по модели, записанной для него, так что вызов, перенесённый
[`[proxy.routing]`](#proxyrouting), оценивается по модели, которая его обслужила. `*` помечает lane,
где была и модель без строки цены, так что её сумма — нижняя оценка.

### Хост WASM-плагинов (`[plugins.wasm]`)

Внешние плагины `.wasm` (P32, решение D6). Этот репозиторий пишет каждый плагин каталога с
нуля и **никогда не включает в себя сторонние блобы `.wasm`** — операторы устанавливают их в
`plugins.wasm.dir` на своей машине. По умолчанию `enabled = false`: встроенные сборки и конфигурация по умолчанию
не загружают WASM. Хост Wasmi и фича Cargo `wasm-host` появятся в T32.2; до тех пор
флаг виден в `rtok config show --sources`, но загрузчика у него нет.

### Бэкенды графа (`[plugins.graph]`)

`backend = "tags"` (по умолчанию) отвечает из индекса tree-sitter. `"lsp"` направляет `symbol` / `callers` /
`impact` / `outline` / `explore` через языковой сервер из `PATH` и отдаёт ответ tags, когда сервер
ответить не может. `"auto"` выбирает режим для каждого проекта и языка: сервер, если для языка проекта
он установлен, иначе tags, а если ни одна грамматика не разбирает проект — текстовый поиск, и каждый
ответ сообщает, какой режим ответил. `"text"` (T329.10) закрепляет простой текстовый поиск: шаблоны со
границами слов для определений и упоминаний по файлам, которые обходит инструмент `search`, внутри
процесса (`rg`, `grep` и `ssh` не запускаются), без графа вызовов, поэтому `dead` и цепочки `to` в
`impact` отвечают «not available in text mode».

`backend_by_language` переопределяет `backend` для одного языка, который определяется по файлу-маркеру
проекта: `rust`, `c`, `typescript`, `dart`, `go`, `python`, `javascript`, `java`, `ruby`, `php`,
`elixir`, `swift` (`go = "tags"` оставляет Go на индексе, пока остальные работают в `auto`). `lsp_timeout_ms` (по умолчанию 40000) — сколько может длиться
одно ожидание сервера, прежде чем запрос откатится на tags. Пошаговая настройка для Rust (rust-analyzer) и
Dart (Dart SDK) и формат ответа `auto`: `docs/lsp.md`.

### Вывод в терминал (`[ui]`)

Собственные строки rtok для человека за терминалом — `ok …`, `… started` / `… stopped`, `warning: …`,
`Error: …`, сводка `graph index`, `--help` — по умолчанию снабжены эмодзи и цветом:
✅ успех (зелёный), 💡 статус (голубой), ❗ предупреждение (жёлтый), ❌ ошибка (красный). Строка с названием операции получает её значок (📚 index, 🚀 start, 🛑 stop, 🔗 link, 🧹 remove, …), выровненный так, что текст после него начинается в одной колонке.

```toml
[ui]
emoji = false   # RTOK_UI_EMOJI=false
color = false   # RTOK_UI_COLOR=false
```

- **Эмодзи** требуют `emoji = true` *и* терминала на этом потоке.
- **Цвет** требует `color = true` *и* потока, который принимает цвет: терминал, `NO_COLOR`
  не задан, `TERM` не `dumb` — или `CLICOLOR_FORCE` / `FORCE_COLOR`, принудительно включающие его для пайпа.
  `color = false` также выключает цвета диффа, слов состояния и уровней логов.
- То, что читают агенты, никогда не меняется: JSON хуков и MCP, отфильтрованный вывод команд, `--json` и
  всё, что пишется в пайп или файл, остаются простыми, байт в байт.
- Цвет `--help` решает сам clap (терминал, `NO_COLOR`, `CLICOLOR_FORCE`): он печатается
  до чтения конфигурации.

## TLS и корпоративные CA

`rtok proxy` и экспорт OpenTelemetry используют одну общую конфигурацию клиента rustls: корни Mozilla
через `webpki-roots`, переданные в reqwest с `use_preconfigured_tls`. Они
не используют верификатор macOS Security.framework. Корпоративные или частные CA:
задайте `SSL_CERT_FILE` с бандлом PEM (соглашение curl). Эти сертификаты
дополняют набор Mozilla. Если переменная задана, отсутствующий, пустой или
не разбираемый файл проваливает запуск с путём в сообщении об ошибке (как в curl).
Без неё используются только корни Mozilla. `rtok hook` никогда не открывает TLS.

## OpenTelemetry

`[otel]` включает экспортёр (`docs/otel.md`). Выключен, пока `endpoint` или
`OTEL_EXPORTER_OTLP_ENDPOINT` не укажет коллектор; на пути хука ничего не выполняется.

## Таблица соответствия (флаги → ключи)

| Подкоманда | Флаг | Ключ |
|-----------|------|-----|
| глобально | `--config <path>` | (выбирает файл; не ключ) |
| глобально | `RTOK_HOME` | (выбирает каталог; только окружение, не флаг clap) |
| чтение | `--json` | `stats.format` для `stats`; в остальных случаях действие (страница `web::model` в виде JSON, не сохраняемый ключ). Для `stats`, `info`, `config show`, `doctor`, `plugins`, `agents list`, `agents sessions`, `agents whoami`, `agents show`, `agents inbox`, `worktree whoami`, `logs`, `demon status`, `otel status` |
| `hook` | `--host` | `hook.host` |
| `proxy` | `--port`, `--upstream`, `--mode`, `--dry-run` | `proxy.port`, `proxy.upstream`, `proxy.mode`, `proxy.dry_run` |
| `web` | `--host`, `--port` | `web.host`, `web.port` (`rtok dashboard` — устаревшее написание) |
| `tui` | `--tab`, `--tick-secs` | `tui.tab`, `tui.tick_secs` |
| `stats` | `--since`, `--plugin`, `--compare`, `--calibrate`, `--cache`, `--price` | `stats.since`, `stats.format`, `stats.plugin`, `stats.baseline`, (`--calibrate`, `--cache` — действия; их настройки — `stats.calibrate_samples`), `stats.price` (`stats.prices.*` — данные) |
| `report` | `--format`, `--out`, `--since`, `--ai` | `report.format`, `report.out`, `report.since`, `report.ai` (`report.budget_tokens` ограничивает `--ai`) |
| `bench` | `--tasks`, `--runs`, `--dry-run`, `--timeout`, `--suite` | `bench.*` |
| `doctor` | `--instructions` | `doctor.instructions` |
| `agents install` | `--dry-run`, `--yes`, `--mode`, `--mcp`, `--proxy`, `--remove`, `--replace`, `--cli`, `--desktop`, `--all` | `setup.*` (`--remove`, `--replace`, `--cli`, `--desktop`, `--all` — действия) |
| `agents remove` | `--dry-run` | `setup.dry_run` (сама команда — это действие `--remove`) |
| `agents list` | — | читает конфигурации хостов и `<bin> --version` (`--json` — строка «чтение») |
| `agents whoami` | — | читает `RTOK_AGENT_ID` и разрешает его через хранилище (T283); нет ключа, нет `setup.*` (`--json` — строка «чтение») |
| `worktree whoami` | — | читает `RTOK_AGENT_ID` и `[worktree] root` (T411); собственного ключа нет (`--json` — строка «чтение») |
| `task init` | `--adapter`, `--prefix` | `tasks.adapter`, `tasks.prefix`: записываются в `.rtok.toml` этой копии репозитория (T441.5) |
| `task export` | `--dry-run`, `--json` | `tasks.export.github.repo`, `tasks.export.linear.team` (T441.13.5, T441.13.6); `--dry-run` — на один вызов (без ключа) |
| `task create` / `list` / `status` | `--description`, `--body-file`, `--parent`, `--status`, `--all`, `--force` | для одного вызова (без ключа): какая задача и какие строки показать; адаптер и префикс выбирает `[tasks]` |
| `agents junk list` / `clear` | `--agent`, `--kind`, `--include review`, `--older-than`, `--session-days`, `--trash`, `--bytes`, `--items`, `--sort`, `--min-size`, `--yes` | на один вызов (без ключа): что один запуск показывает или удаляет; `--session-days` это `agents.junk.stale_session_days` на один запуск; `agents.junk.keep_logs_days`, `.temp_min_age_hours`, `.crash_dump_min_age_days`, `.stale_worktree_days`, `.exclude`, `.extra` без флага |
| `agents usage` | `--source`, `--host`, `--since`, `--until`, `--daily` / `--monthly`, `--tz` | `agents.usage.source`, `.hosts`, `.since`, `.until`, `.period`, `.tz`, а также `.dirs.<host>` без флага (`--unpriced` выбирает вид одного вызова, `--json` — строка «чтение») |
| `agents sessions` | `--all` | (действие: также перечисляет завершённые сессии; live или idle — по `agents.idle`) |
| `agents show` | — | разрешает префикс id через хранилище (T284); live или idle — по `agents.idle` (`--json` — строка «чтение») |
| `agents status` | — | записывает текст статуса вызывающего агента (`RTOK_AGENT_ID`), ≤ 120 символов (T284); нет ключа |
| `agents send` | `--all-live` | для отдельного вызова (нет ключа, нет `setup.*`): одно сообщение каждому живому агенту проекта вызывающего (T287); что считается «живым», решает `[agents] idle` |
| `agents inbox` | `--unread` | для отдельного вызова (нет ключа, нет `setup.*`): какие строки показывает одно чтение (T287); `--json` — строка «чтение» |
| `expand` | `--lines`, `--grep` (регулярное выражение, с откатом к буквальному поиску; совпадения печатаются как `N:line`), `--context N` (строки вокруг каждого совпадения grep, окна объединяются через `--`) | для отдельного вызова (нет ключа); `expand.max_lines` ограничивает; `expand.max_rate` — потолок для отчёта (T22.5) |
| `filter` | `--cmd` | `filter.cmd` |
| `config init`, `config set`, `memory import`, `graph index` | `--dry-run` | (действие: показывает изменение как git diff и ничего не записывает) |
| `memory export` | `--project` | для отдельного вызова (нет ключа): сужает один дамп до заметок проекта |

Тест покрытия (T12.4) обходит дерево команд clap и падает, если не позиционный флаг
встречается без ключа в `crates/rtok-config/default.toml`, поэтому эта таблица не может незаметно разойтись с кодом.

## Примеры переменных окружения

```bash
RTOK_PROXY_MODE=compress rtok proxy
RTOK_PLUGINS_READ_ALLOW_PATHS=/opt/src,/srv/lib rtok mcp
RTOK_PLUGINS_WASM_ENABLED=true rtok config show --sources
RTOK_STATS_SINCE=7d rtok stats
RTOK_CONFIG=./ci-config.toml rtok bench --dry-run
```

## Отладочный лог (`RUST_LOG`)

`[log]` — это лог оператора: файл с настроенным уровнем плюс строки `logs`. Для отладки
есть второй поток, только в stderr, за фасадом `log` и `env_logger` (T225). Он выключен,
пока не задан `RUST_LOG`, поэтому по умолчанию ни один запуск хука, `mcp` или `proxy` не печатает ничего нового:

```bash
RUST_LOG=rtok=debug rtok stats                       # argv, затем каждая строка [log] по мере записи
RUST_LOG=rtok::log=info rtok proxy                   # только зеркалируемый поток [log]
RUST_LOG=rtok=debug RUST_LOG_STYLE=never rtok mcp    # без цвета; stdout остаётся каналом MCP
```

Зеркало игнорирует `[log] level`: файл хранит `info`, а stderr показывает то, что запрашивает `RUST_LOG`.
Переменная называется `RUST_LOG`, а не `RTOK_LOG`: имена `RTOK_<SECTION>_<KEY>` принадлежат
уровню окружения выше, и `RTOK_LOG` конфликтовал бы с таблицей `[log]`.

`just logs [flags]` открывает файл в [tailspin](https://github.com/bensadeh/tailspin) (`tspin`,
закреплён в `mise.toml`), который подсвечивает уровни, даты, числа и пути; `just logs -f` следит
за ним. Отладочный поток передаётся по пайпу так же: `RUST_LOG=rtok=debug rtok stats 2>&1 >/dev/null | tspin`.

`rtok logs` и `rtok logs watch` сами используют tailspin (T225.1). При `[log] tspin = "auto"`,
значении по умолчанию, пронумерованные строки проходят через `tspin --print`, когда stdout — терминал, а `tspin`
есть в `PATH`; `"always"` делает так и для пайпа, `"off"` сохраняет собственные цвета rtok. Без `tspin`
вывод остаётся прежним. `rtok logs export` и `--json` никогда через него не проходят.

## Почему один файл, а не только флаги

Хуки запускаются хостом с фиксированной командной строкой; единственный способ их настроить —
файл. Прокси и MCP-сервер работают часами; перезапускать их ради изменения флага — это
регрессия. А бенчмарку нужны две полные воспроизводимые конфигурации — то есть по файлу
на конфигурацию, а не история оболочки.

## Устаревшие ключи

`[dashboard]` сворачивается в `[web]` (T21.3). `core.inject_budget_tokens` сворачивается в
`plugins.inject.budget_tokens` (T12.1). `core.log_file` / `log_level` / `log_to_db` сворачиваются в
`[log].path` / `level` / `to_db` (T24.5, D26). Каждый печатает одно предупреждение при загрузке и затем
отбрасывается; `rtok config validate` отклоняет их, потому что их нет в эталонной схеме.

`setup.restart_prompt_timeout_seconds` выведен из употребления (T138): вопрос о перезапуске после установки — это
подтверждение `inquire`, которое ждёт ответа (Enter, Esc или Ctrl-C означают «нет»). Файл, который всё ещё задаёт
этот ключ, получает предупреждение о неизвестном ключе при загрузке и ошибку от `rtok config validate`.
