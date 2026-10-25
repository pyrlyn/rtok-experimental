---
lang: ru
---

# Агенты

Каждый хост, который настраивает `rtok agents install <host>`, — по приложениям: что rtok в него записывает и какие плагины rtok затем доходят до этого приложения.

- **Hooks**, **MCP**, **Proxy**, **Plugin** — четыре модуля, которые может включать установка. `yes` записывается обычной установкой; флаг (`--proxy`, `--yes`) означает, что модуль записывается только с этим флагом; `—` означает, что хост не может его нести. README хоста (ссылка в столбце Host) объясняет причину каждого `—`.
- **MCP** — это запись `rtok` в собственном файле конфигурации приложения (D33): установка и обновление всегда её записывают, удаляет её только `remove`, и плагин никогда её не заменяет. Запись запускает `rtok mcp --host <id>` (id хоста из этой таблицы), чтобы процесс MCP знал свой хост; `agents update` добавляет `--host` к записи, которую более старый rtok записал без него, а `remove` по-прежнему удаляет любую из форм.
- **Plugin** — это каталог из `plugins/<host>/`, который rtok подключает ссылкой в хост (Cursor и ZCode: хуки; OpenCode и pi: путь вызова bash). Там, где приложение показало бы сервер плагина рядом с записью конфигурации (Claude Code и Desktop, Cursor, Copilot, Codex, VS Code, ZCode, Kimi, Grok), плагин rtok не поставляет MCP-сервер; расширение Gemini его сохраняет, а запись в `settings.json` его переопределяет.
- **rtok plugins reached** перечисляет каждый плагин из каталога, у которого есть хотя бы один интерфейс (хук, путь вызова bash, MCP, прокси), который может нести один из модулей выше. Плагины с пометкой `(off)` по умолчанию выключены, и для них нужно `[plugins.<id>] enabled = true`.

Проверить, что установлено на этой машине, можно командой `rtok agents list`; `rtok doctor` сообщает о тех же модулях и предупреждает, когда в конфигурации установленного хоста нет записи MCP `rtok` или она устарела, называя команду `rtok agents install <host>` (с `--cli` или `--desktop`), которая её запишет.

Когда rtok не находит приложение хоста (ни в обычных папках приложений, ни в PATH), `rtok agents install <host>`, `update <host>` и `info <host>` в терминале спрашивают путь к нему: CLI-бинарник или десктопное приложение. rtok проверяет путь, показывает агента, путь и файлы конфигурации хоста и спрашивает `Is this correct? [y/N]`; «да» выполняет команду с этим путём, «нет» спрашивает снова, пустой ответ останавливает. Затем rtok предлагает сохранить путь как `[setup.paths] <host>` (см. `docs/config.md`); каждый поиск сначала читает этот ключ. Без терминала ничего не спрашивается: `install` завершается ошибкой, в которой назван ключ, `rtok config set setup.paths.<host> <path>`.

`rtok doctor` также сообщает о сломанных хуках, дублирующихся хуках и дублирующихся записях MCP в файлах конфигурации хостов. `rtok doctor --fix` их убирает. В терминале он открывает чек-лист всех удаляемых пунктов: номер переключает пункт, `k N` оставляет вместо этого копию N дубликата, `d` показывает diff каждого файла, `y` запрашивает последнее подтверждение и записывает (после резервной копии в `.backup_configs/`), `q` выходит. Пункты в общем файле проекта изначально не выбраны, поскольку изменение затронет ваших коллег. Без терминала `--fix` печатает diff и ничего не записывает; `--fix --yes` записывает выбор по умолчанию без вопросов; `--only broken-hooks|duplicate-hooks|duplicate-mcp` (можно повторять) ограничивает набор, а `--agent <host>` ограничивает одним хостом. Оставленная копия дубликата, собственные файлы плагина и собственные записи rtok никогда не удаляются, а каждый байт за пределами удалённых записей остаётся как был. Запись `rtok`, которую пишет установка, всегда остаётся оставленной копией. Такое же имя `rtok` в другой области и копия плагина (расширение Gemini) выводятся только как сведения, потому что хост сам объединяет или перекрывает их (для копии плагина `rtok agents update <host>` обновляет плагин); к удалению предлагается только написанная вручную копия под другим именем, которая тоже запускает `rtok mcp`, например `rtok-mcp`. Во вкладке Doctor в `rtok tui` тот же список: `f` открывает его, `Space` переключает запись, `k` оставляет эту копию, `Enter` просит подтверждение, `y` записывает; любая другая клавиша отменяет.

`rtok doctor` также предупреждает об установленных скилах rtok (`rtok`, `rtok-tasks`, `rtok-worktrees` в папке скилов хоста), которые старше тех, что поставляет этот rtok: по строке `WARN` на каждую копию. `rtok doctor --fix --only stale-skills` обновляет их той же копией, что делает `rtok agents update`: в терминале показывает список и спрашивает `[y/N]`, без терминала только показывает список, а `--yes` обновляет без вопросов. Об отсутствующем скиле не сообщается, а папка, которая не принадлежит rtok, никогда не трогается.

После обновления rtok команда `rtok agents update` приводит все установленные хосты к актуальному состоянию; для Claude Code решение принимается по версии плагина — см. [Версии и обновления плагинов](plugin-versions.md).

<!-- agents-table:start (generated by tests/agents_doc.rs) -->
| Host | App | Kind | Hooks | MCP | Proxy | Plugin | rtok plugins reached |
| --- | --- | --- | --- | --- | --- | --- | --- |
| [`claude`](https://github.com/pyrlyn/rtok/blob/main/src/agents/claude/README.md) | Claude Code | CLI | yes | yes | `--proxy` | yes | measure, cmd, read, json_tree (off), archive, proxy, inject, guard, memory, graph, toon, compress, docs (off) |
| [`claude`](https://github.com/pyrlyn/rtok/blob/main/src/agents/claude/README.md) | Claude Desktop | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`cursor`](https://github.com/pyrlyn/rtok/blob/main/src/agents/cursor/README.md) | Cursor CLI | CLI | yes | yes | — | yes | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`cursor`](https://github.com/pyrlyn/rtok/blob/main/src/agents/cursor/README.md) | Cursor | Desktop | yes | yes | — | yes | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`codex`](https://github.com/pyrlyn/rtok/blob/main/src/agents/codex/README.md) | Codex | CLI | yes | yes | `--proxy` | yes | measure, cmd, read, json_tree (off), archive, proxy, inject, guard, memory, graph, toon, compress, docs (off) |
| [`codex`](https://github.com/pyrlyn/rtok/blob/main/src/agents/codex/README.md) | Codex App | Desktop | yes | yes | `--proxy` | yes | measure, cmd, read, json_tree (off), archive, proxy, inject, guard, memory, graph, toon, compress, docs (off) |
| [`opencode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/opencode/README.md) | OpenCode | CLI | — | yes | yes | yes | measure, cmd, read, json_tree (off), archive, proxy, guard, memory, graph, toon, compress, docs (off) |
| [`opencode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/opencode/README.md) | OpenCode Desktop | Desktop | — | yes | yes | yes | measure, cmd, read, json_tree (off), archive, proxy, guard, memory, graph, toon, compress, docs (off) |
| [`kilo`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kilo/README.md) | Kilo Code CLI | CLI | — | yes | — | yes | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`kilo`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kilo/README.md) | Kilo Code for VS Code | Desktop | — | yes | — | yes | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`pi`](https://github.com/pyrlyn/rtok/blob/main/src/agents/pi/README.md) | pi | CLI | — | — | — | yes | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`omp`](https://github.com/pyrlyn/rtok/blob/main/src/agents/omp/README.md) | oh my pi | CLI | — | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`zcode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/zcode/README.md) | ZCode | Desktop | yes | yes | — | yes | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`kimi`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kimi/README.md) | Kimi Code | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`kimi`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kimi/README.md) | Kimi Code Desktop | Desktop | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`grok`](https://github.com/pyrlyn/rtok/blob/main/src/agents/grok/README.md) | Grok Build | CLI | — | yes | — | `--yes` | cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`vscode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/vscode/README.md) | VS Code | Desktop | — | yes | — | `--yes` | cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`vscode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/vscode/README.md) | VS Code - Insiders | Desktop | — | yes | — | `--yes` | cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`copilot`](https://github.com/pyrlyn/rtok/blob/main/src/agents/copilot/README.md) | Copilot CLI | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`copilot`](https://github.com/pyrlyn/rtok/blob/main/src/agents/copilot/README.md) | GitHub Copilot | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`commandcode`](https://github.com/pyrlyn/rtok/blob/main/src/agents/commandcode/README.md) | Command Code CLI | CLI | yes | yes | — | yes | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`aider`](https://github.com/pyrlyn/rtok/blob/main/src/agents/aider/README.md) | aider | CLI | — | — | `--proxy` | — | measure, json_tree (off), archive, proxy, toon, compress |
| [`windsurf`](https://github.com/pyrlyn/rtok/blob/main/src/agents/windsurf/README.md) | Windsurf | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`zed`](https://github.com/pyrlyn/rtok/blob/main/src/agents/zed/README.md) | Zed CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`zed`](https://github.com/pyrlyn/rtok/blob/main/src/agents/zed/README.md) | Zed | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`cline`](https://github.com/pyrlyn/rtok/blob/main/src/agents/cline/README.md) | Cline CLI | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`cline`](https://github.com/pyrlyn/rtok/blob/main/src/agents/cline/README.md) | Cline for VS Code | Desktop | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`gemini`](https://github.com/pyrlyn/rtok/blob/main/src/agents/gemini/README.md) | Gemini CLI | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`codewhale`](https://github.com/pyrlyn/rtok/blob/main/src/agents/codewhale/README.md) | CodeWhale | CLI | yes | yes | — | — | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`mimo`](https://github.com/pyrlyn/rtok/blob/main/src/agents/mimo/README.md) | MiMo Code | CLI | — | yes | — | yes | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`mimo`](https://github.com/pyrlyn/rtok/blob/main/src/agents/mimo/README.md) | MiMo Desktop | Desktop | — | yes | — | yes | measure, cmd, read, json_tree (off), archive, guard, memory, graph, toon, docs (off) |
| [`antigravity`](https://github.com/pyrlyn/rtok/blob/main/src/agents/antigravity/README.md) | Antigravity CLI | CLI | — | — | — | `--yes` | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`antigravity`](https://github.com/pyrlyn/rtok/blob/main/src/agents/antigravity/README.md) | Antigravity | Desktop | — | — | — | `--yes` | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`devin`](https://github.com/pyrlyn/rtok/blob/main/src/agents/devin/README.md) | Devin CLI | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`devin`](https://github.com/pyrlyn/rtok/blob/main/src/agents/devin/README.md) | Devin | Desktop | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`roo`](https://github.com/pyrlyn/rtok/blob/main/src/agents/roo/README.md) | Roo Code | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`qwen`](https://github.com/pyrlyn/rtok/blob/main/src/agents/qwen/README.md) | Qwen Code | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`codebuddy`](https://github.com/pyrlyn/rtok/blob/main/src/agents/codebuddy/README.md) | CodeBuddy Code | CLI | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`codebuddy`](https://github.com/pyrlyn/rtok/blob/main/src/agents/codebuddy/README.md) | WorkBuddy | Desktop | yes | yes | — | `--yes` | measure, cmd, read, json_tree (off), archive, inject, guard, memory, graph, toon, docs (off) |
| [`goose`](https://github.com/pyrlyn/rtok/blob/main/src/agents/goose/README.md) | Goose CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`goose`](https://github.com/pyrlyn/rtok/blob/main/src/agents/goose/README.md) | Goose | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`droid`](https://github.com/pyrlyn/rtok/blob/main/src/agents/droid/README.md) | Factory Droid | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`kiro`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kiro/README.md) | Kiro CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`kiro`](https://github.com/pyrlyn/rtok/blob/main/src/agents/kiro/README.md) | Kiro IDE | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`junie`](https://github.com/pyrlyn/rtok/blob/main/src/agents/junie/README.md) | Junie CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`amp`](https://github.com/pyrlyn/rtok/blob/main/src/agents/amp/README.md) | Amp | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`warp`](https://github.com/pyrlyn/rtok/blob/main/src/agents/warp/README.md) | Warp | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`continue`](https://github.com/pyrlyn/rtok/blob/main/src/agents/continue/README.md) | Continue CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`continue`](https://github.com/pyrlyn/rtok/blob/main/src/agents/continue/README.md) | Continue | Desktop | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`crush`](https://github.com/pyrlyn/rtok/blob/main/src/agents/crush/README.md) | Crush | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`openhands`](https://github.com/pyrlyn/rtok/blob/main/src/agents/openhands/README.md) | OpenHands | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`augment`](https://github.com/pyrlyn/rtok/blob/main/src/agents/augment/README.md) | Augment Auggie CLI | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
| [`reasonix`](https://github.com/pyrlyn/rtok/blob/main/src/agents/reasonix/README.md) | Reasonix | CLI | — | yes | — | — | read, json_tree (off), archive, memory, graph, toon, docs (off) |
<!-- agents-table:end -->

Таблица генерируется из кода хостов (`src/agents/<host>/mod.rs` и каталог плагинов) тестом `tests/agents_doc.rs`, который падает, если она устарела. После изменения хоста, добавления нового или изменения интерфейсов плагина перегенерируйте её:

```bash
RTOK_BLESS=1 mise exec -- cargo test --test agents_doc
```

Столбец **rtok plugins reached** вычисляется по объявленным интерфейсам: плагин указывается, когда
хост поддерживает интерфейс, на котором плагин отвечает. Это не доказывает, что каждое событие, нужное
плагину, зарегистрировано на этом хосте, — проверка этого для каждого хоста — задача T70.7 в `plan.md`.

Claude Code также получает хук `PreToolUse` на `Skill` (T62.1). Он бездействует, пока не задано `[plugins.guard] skills = true`; тогда `SKILL.md` больше `skill_max_bytes` архивируется, а вызов отклоняется с картой заголовков файла и указателем `rtok expand <id>`. Навык, сжатый таким образом, не применяет свой frontmatter, поэтому навыки с `allowed-tools`, `model`, `context` или `agent` всегда загружаются целиком.

## Куда доходит плагин хоста, а собственные интерфейсы rtok — нет

У rtok три собственных интерфейса (хук, MCP, прокси), и у каждого есть жёсткая граница:

- **Хук PostToolUse не может изменить результат инструмента**, только добавить контекст. Поэтому в Claude Code
  сокращается только `Bash` — он переписывается в `rtok run` *до* выполнения. Результат `Read`,
  `Grep` или стороннего MCP попадает в контекст целиком.
- **Живая зона `archive` работает внутри `rtok proxy`**. Хост без настройки базового URL никогда
  не сокращает старый результат инструмента, как бы долго ни длилась сессия.
- **Хост без событий хуков не получает ни одного плагина хуков**, поэтому у `inject` нет пути внутрь. `guard` доходит до pi и OpenCode через `rtok guard check` в подключённом плагине (T70.5).

**Плагин хоста** — каталог в `plugins/<host>/`, который подключает ссылкой `rtok agents install <host>`, —
находится внутри процесса хоста и не ограничен этими границами. Там, где API плагинов хоста
богаче его API хуков, плагин — единственный путь к нужной возможности. Задокументированные положения API
по обзору от 2026-09-18 (`research.md` §15):

| Что предлагает API плагинов | pi | OpenCode | Cursor |
| --- | --- | --- | --- |
| Заменить результат любого инструмента | `tool_result` | сегодня bash; другие инструменты не задокументированы | только результаты MCP |
| Заблокировать вызов с причиной, которую читает модель | `tool_call` | `tool.execute.before` | `before…Execution` |
| Переписать сообщения, отправляемые модели | `context` | — | — |
| Действовать при компакции | `session_before_compact` | `experimental.session.compacting` | `preCompact` (только наблюдение) |
| Зарегистрировать инструмент без MCP | `pi.registerTool` | — | — |

Из этого вытекают два следствия, определяющие открытую работу в `plan.md`: в pi вся живая зона `archive` может работать
без прокси (T70.2), а результаты MCP, которые хост запустил сам, можно сокращать там, где это
позволяет API плагинов (T70.4), — ни то ни другое недостижимо из `rtok mcp`, `rtok hook` или `rtok proxy`.
Плагины Claude Code и ZCode несут те же события хуков, что и их файлы настроек, без более богатого
API; у Codex, Claude Desktop, Windsurf, Zed, Kimi и Copilot каталога плагина нет. Для всех
них прокси остаётся единственным путём.

Каждая из этих задач начинается с повторной сверки API хоста с его актуальной
документацией: таблица выше — обзор документации вендоров, а не измерение.

## Какие хосты читает `rtok agents usage`

`rtok agents usage --source logs` суммирует токены по собственным файлам сессий каждого хоста (каталоги задаются в `[agents.usage.dirs]` в `docs/config.md`, а `research.md`, раздел 30, указывает источник каждого формата).

| Host | Reads | Status |
| --- | --- | --- |
| Claude Code, Codex | транскрипты в `[stats]` | поддерживается |
| OpenCode, Kilo | строки `message` в файле SQLite хоста, открытом только для чтения | поддерживается |
| Copilot CLI | итоги по моделям каждой завершённой сессии | поддерживается, по сессиям |
| Gemini CLI | `tokens` каждого ответа в логах чатов | поддерживается |
| pi | `usage` каждой записи в логах сессий | поддерживается |
| Kimi Code | строки `usage.record` в wire-логе каждого агента | поддерживается |
| Droid | нет: поля токенов в его файле настроек не задокументированы | `unsupported` |
| Grok | нет: xAI документирует `grok usage`, который rtok не запускает, а не файлы | `unsupported` |
| ZCode | нет: записи сессий не задокументированы | `unsupported` |
| Antigravity | нет: его локальные данные не задокументированы | `unsupported` |

Хост со статусом `unsupported`, у которого в каталоге есть файлы, указывается в отчёте в разделе `skipped`, чтобы отсутствующий агент был виден, а не молча считался нулём.

## Устаревшие плагины

`rtok agents outdated` перечисляет только хосты, где установлен плагин rtok и его версия
старше запущенного бинарника rtok (по старшинству SemVer; метаданные сборки того же релиза
по-прежнему считаются актуальными). Хосты, которые актуальны, не установлены или новее, пропускаются. `rtok agents
update --check` печатает тот же отчёт.

```bash
rtok agents outdated              # каждый хост из таблицы выше
rtok agents outdated claude,cursor
rtok agents outdated --json
rtok agents outdated --exit-code  # код выхода 10, если что-то устарело (CI)
```

Когда все установленные плагины соответствуют бинарнику, команда печатает, сколько их установлено, и
версию rtok; когда не установлено ни одного, она печатает `no rtok plugins installed`. Проверка
только локальная (квитанция, файл версии, записи хостов) — без сети и без CLI хоста.
Подробнее — в [Версии плагинов: Список устаревших плагинов](plugin-versions.md#список-устаревших-плагинов).

## Мусор

`rtok agents junk list` показывает для rtok и каждого установленного хоста его папки с размерами, виды мусора внутри них и сколько освободит `rtok agents junk clear`. `clear` — пробный прогон до `--yes`.

Обе команды печатают элементы, из которых сложены итоги, чтобы точный набор можно было проверить до удаления. Элемент — одна строка: путь (ссылка в терминале, который их поддерживает, с `~` для домашней папки), размер, когда его использовали в последний раз (самый новый файл внутри, например `3 weeks ago (2026-09-09 14:02)`), почему это мусор и, если элемент не будет очищен, почему (`review kind: add --include review`, `agent running`).

- `--items N|all` задаёт, сколько элементов показывать для каждого вида (по умолчанию 10, затем `+N more (размер)`); `--items 0` печатает только итоги.
- `--sort size|last-used|path` упорядочивает элементы внутри вида: сначала крупные (по умолчанию), сначала давно не используемые или по пути. Элемент с нечитаемым временем считается свежим и идёт последним. Сами агенты всегда идут по месту, которое освободит `clear`, от большего к меньшему.
- `--min-size РАЗМЕР` скрывает более мелкие элементы. У `clear` он ещё и сужает план, так что удаляется ровно то, что показал пробный прогон.
- `--json` всегда содержит все элементы: `path`, `size_bytes`, `last_used` (секунды Unix или `null`), `reason`, `will_clear` и, если элемент не очищается, `skip_reason`.

`rtok doctor` добавляет строку `junk` с местом, которое `clear` освободит по умолчанию, и указывает на `rtok agents junk list`, если оно больше гигабайта. Страница Hosts в `rtok web` и `rtok tui` показывает тот же список.

На странице Hosts в `rtok web` список выглядит как карточка `junk`: по каждому агенту и виду — число элементов и размеры, а также строки `Freed by clear` и `Freed with --include review`. Кнопка **Clear safe junk** запускает пробный прогон `clear` по `safe`-видам всех агентов (никогда не по видам `review`, `explicit` и `never` и никогда не в корзину), показывает, что будет удалено, и ничего не удаляет, пока вы не нажмёте **Confirm**. Удаляются только показанные вам элементы, и каждый проверяется заново перед удалением, поэтому файлы работающего агента, изменившаяся папка, симлинк и защищённый файл остаются и попадают в отчёт. На вкладке Hosts в `rtok tui` клавиша `c` показывает тот же пробный прогон, а `y` удаляет перечисленное; любая другая клавиша отказывается и оставляет все файлы как были. Результат заканчивается строкой `Freed X of Y planned`.
