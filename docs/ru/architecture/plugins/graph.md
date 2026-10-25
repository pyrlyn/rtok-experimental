---
lang: ru
---

# Плагин graph

![graph: индекс символов tree-sitter, который поддерживается в актуальном состоянии инкрементально вне пути хука, отвечает на инструменты MCP symbol, callers, impact, outline, explore, graph_diff и graph_export, а также на CLI rtok graph](../../../architecture/plugins/graph.svg)

Источник: [`graph.d2`](../../../architecture/plugins/graph.d2). Документация плагина: [README](../../../../src/plugins/graph/README.md). Оглавление: [все диаграммы](../README.md).

`graph` заменяет несколько серверов графа кода несколькими ограниченными инструментами поверх индекса символов, который rtok строит сам по тегам tree-sitter: определения, места ссылок и для каждой ссылки самое внутреннее охватывающее определение, то есть ребро вызова, по которому ходят `callers` и `impact`. Это индекс тегов, а не граф вызовов с разрешением типов. Индексация инкрементальна и никогда не выполняется на пути хука; каждый ответ ограничен `plugins.graph.max_tokens`, а полный текст остаётся доступным через `expand <id>`.

## Пошагово

1. **Вызов входит.** Инструменты MCP `symbol`, `callers`, `impact`, `outline`, `explore`, `graph_diff` и `graph_export` перечислены в `GraphPlugin::mcp_tools` и направляются в `invoke` в `src/mcp.rs`. Эта ветка регистрирует проект (`auto_add_project`), открывает запись `graph_events` (`events::Call`), разрешает область проекта через `scope::resolve` (запрошенный проект плюс проекты, с которыми он связан), а затем добавляет в начало ответа `health::notice`. Подкоманды `rtok graph` (`index`, `status`, `dead`, `impact`, `review`, `affected`, `diff`, `export`, `projects`) попадают в тот же код из `src/cli/graph.rs`.
2. **Индексация по запросу.** Каждый запрос сначала вызывает `index_for`: при `auto_index = true` он запускает `index::run`, иначе `index::ensure` индексирует корень без строк один раз. `walk::Matcher` обходит дерево по правилам игнорирования, настройкам включения и исключения и карте расширений; стат-проверка пропускает файл, у которого не изменились mtime и размер, а sha-проверка — файл, у которого не изменился sha256. Изменённые файлы разбираются параллельно вместе с запросами тегов, общими с плагином `read`, а каждая запись остаётся в одном потоке: `replace_symbol_files`, `delete_symbols_missing`, `rebuild_symbol_idf`. Изменившийся отпечаток экстрактора удаляет строки корня и переиндексирует его с нуля.
3. **Граф файлов обновляется.** Когда запуск изменил корень, `rank::refresh` перестраивает документ PageRank файлов (`file_rank`, с парами совместных изменений git из `kv` как дополнительными рёбрами), чтобы SessionStart мог ранжировать файлы, не сканируя `symbols`.
4. **Свежесть без разбора.** `PostToolUse` для `Edit` или `Write` вызывает только `mark_symbols_stale`, который удаляет строки этого файла и отмечает его в `symbol_stale`. Когда `[plugins.graph] watch` не равно `off`, `watch::run_scope` — поток внутри `rtok mcp`, который вызывает `index::run_changed` по событиям файлов, так что писатель по-прежнему один.
5. **Веерная рассылка по проектам.** `graph::call` передаёт инструмент в `scope::symbols`, `callers`, `impact`, `explore` или `outline`, которые обходят участников области и объединяют ответы под одним ограничением.
6. **Выбор бэкенда.** Для каждого проекта `text_or` сначала проверяет `text::applies`; иначе `door` выбирает по `backend`: `tags` отвечает из индекса; `lsp` и `auto` спрашивают запись возможностей (`capability::ask`, проверяется один раз) и вызывают `lsp::Session`, откатываясь к tags с заголовком `(tags; lsp: <reason>)`, когда сервер отсутствует, мёртв или ничего не возвращает для имени, которое индекс знает. `text` отвечает поиском по регулярному выражению внутри процесса с заголовком `(text)`.
7. **Ответы tags.** `symbol_tags` печатает определения с исходным кодом до `body_lines`; `callers_tags` группирует строки ссылок по вызывающему определению (имена неиспользуемых определений не перечисляются); `impact_tags` обходит рёбра вызовов в ширину до `depth`, добавляет партнёров по совместным изменениям git и, сверх `impact_tokens`, группирует файлы через `blast.rs`; `explore_tags` разрешает идентификаторы запроса в определения, добавляет цепочки вызовов между ними и по одной строке влияния на каждый; `outline` переиспользует карту из `read`.
8. **Ограничение.** `cap_kind` архивирует текст сверх `max_tokens`, печатает начало плюс `N more, expand <id>` и записывает `Measurement` только тогда, когда ответ был сокращён. Сохранённый HEAD, отличающийся от текущего, добавляет префикс `index <old> head <new>`.
9. **Другие читатели.** `graph_diff` (`diff::call`) сравнивает символы с git-ссылками, которые читаются из объектной базы git; `graph_export` (`export::call`) выдаёт очищенный JSON `rtok.graph.v1` (CLI может нарисовать его через `draw.rs`); `review` и `affected` ранжируют git-diff; `drill`, `diff::page` и кадры экспорта питают страницы графа в web и TUI.
10. **Карта репозитория.** При SessionStart, если `map_tokens > 0`, `repo_map` предлагает `rank::map` (или самые часто упоминаемые имена) как `Injection` приоритета 1, который бюджетирует `inject`.

## Где это находится

| Блок | Код |
| --- | --- |
| Плагин `GraphPlugin`, `mcp_tools`, `post_tool`, `session_start` | `src/plugins/graph/mod.rs` |
| Маршрутизация MCP, `events::Call`, `health::notice` | `src/mcp.rs` (`invoke`), `src/plugins/graph/events.rs`, `health.rs` |
| `rtok graph` | `src/cli/graph.rs` (`GraphCmd`) |
| `index_for`, `index::run`, `run_changed`, `ensure` | `src/plugins/graph/mod.rs`, `src/plugins/graph/index.rs` |
| `walk::Matcher` | `src/plugins/graph/walk.rs` |
| `rank::refresh`, `rank::map` | `src/plugins/graph/rank.rs`, `cochange.rs` |
| `watch::run_scope` | `src/plugins/graph/watch.rs` |
| `scope::*` | `src/plugins/graph/scope.rs` (`resolve`, `symbols`, `callers`, `impact`, `explore`) |
| Выбор бэкенда | `src/plugins/graph/mod.rs` (`text_or`, `lsp_or_tags`, `door`), `capability.rs`, `lsp.rs`, `text.rs` |
| Бэкенды tags | `src/plugins/graph/mod.rs` (`symbol_tags`, `callers_tags`, `impact_tags`, `explore_tags`), `blast.rs` |
| `cap_kind` | `src/plugins/graph/mod.rs` |
| diff, export, review | `diff.rs`, `export.rs`, `draw.rs`, `review.rs`, `drill.rs` |
| Хранилище символов | `crates/rtok-store/src/symbols.rs`, `symbol_graph.rs` (таблицы: `symbols`, `symbol_stale`, `extractor`, `file_rank`, `kv`, `graph_events`) |

## Инварианты

- Только нативно: индекс строится здесь из тегов tree-sitter; никакой внешний инструмент графа не запускается и не линкуется.
- Индексация никогда не выполняется на пути хука; PostToolUse(Edit|Write) лишь помечает файл устаревшим, а один писатель владеет хранилищем (наблюдатель — поток в `rtok mcp`).
- Каждый ответ ограничен `plugins.graph.max_tokens` и несёт id архива, когда обрезан; строка `Measurement` существует, только когда ответ был сокращён.
- Строки `symbols` привязаны к каноническому корню, поэтому одно хранилище содержит много репозиториев.
- Бэкенд, который не может ответить, откатывается к индексу тегов, а ответ сообщает, какой бэкенд ответил.
- Ключи конфигурации: `[plugins.graph] enabled`, `max_tokens`, `body_lines`, `impact_tokens`, `map_tokens`, `map_rank`, `auto_index`, `backend`, `watch`.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Ход MCP](../mcp-flow.md) · [плагин read](read.md) · [плагин guard](guard.md) · [плагин inject](inject.md)
