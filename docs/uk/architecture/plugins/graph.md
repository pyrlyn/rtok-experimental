---
lang: uk
---

# Плагін graph

![graph: індекс символів tree-sitter, який оновлюється інкрементально поза шляхом хука, відповідає на інструменти MCP symbol, callers, impact, outline, explore, graph_diff і graph_export та на команди CLI rtok graph](../../../architecture/plugins/graph.svg)

Джерело: [`graph.d2`](../../../architecture/plugins/graph.d2). Документація плагіна: [README](../../../../src/plugins/graph/README.md). Індекс: [усі діаграми](../README.md).

`graph` замінює кілька серверів графа коду кількома обмеженими інструментами над індексом
символів, який rtok будує сам за тегами tree-sitter: визначення, місця посилань і для кожного
посилання найглибше визначення, що його містить. Це ребро виклику, яким ходять `callers` та
`impact`. Це індекс тегів, а не граф викликів з визначенням типів. Індексація інкрементальна і
ніколи не виконується на шляху хука; кожна відповідь обмежена `plugins.graph.max_tokens`, а
повний текст лишається доступним через `expand <id>`.

## Покроково

1. **Надходить виклик.** Інструменти MCP `symbol`, `callers`, `impact`, `outline`, `explore`,
   `graph_diff` і `graph_export` перелічує `GraphPlugin::mcp_tools` і маршрутизує `src/mcp.rs`
   `invoke`. Ця гілка реєструє проєкт (`auto_add_project`), відкриває запис `graph_events`
   (`events::Call`), визначає область проєкту через `scope::resolve` (запитаний проєкт плюс
   проєкти, з якими він пов'язаний) і пізніше додає на початок `health::notice`. Підкоманди
   `rtok graph` (`index`, `status`, `dead`, `impact`, `review`, `affected`, `diff`, `export`,
   `projects`) потрапляють у той самий код із `src/cli/graph.rs`.
2. **Індексація за запитом.** Кожен запит спершу викликає `index_for`: якщо `auto_index = true`,
   він запускає `index::run`, інакше `index::ensure` індексує корінь без рядків рівно один раз.
   `walk::Matcher` обходить дерево за правилами ігнорування, налаштуваннями включення та виключення
   і мапою розширень; шлюз stat пропускає файл, чиї mtime і розмір не змінилися, а шлюз sha
   пропускає файл, чий sha256 не змінився. Змінені файли розбираються паралельно разом із
   запитами тегів, спільними з плагіном `read`, а кожен запис іде в одному потоці:
   `replace_symbol_files`, `delete_symbols_missing`, `rebuild_symbol_idf`. Змінений відбиток
   екстрактора скидає рядки кореня і запускає повторну індексацію з нуля.
3. **Граф файлів оновлюється.** Коли прогін змінив корінь, `rank::refresh` перебудовує документ
   PageRank файлів (`file_rank`, з парами спільних змін git із `kv` як додатковими ребрами), щоб
   SessionStart міг ранжувати файли, не скануючи `symbols`.
4. **Свіжість без розбору.** `PostToolUse` для `Edit` або `Write` лише викликає
   `mark_symbols_stale`, який скидає рядки цього файлу і відмічає його в `symbol_stale`. Коли
   `[plugins.graph] watch` не `off`, `watch::run_scope` є потоком усередині `rtok mcp`, що
   викликає `index::run_changed` на подіях файлів, тож писач лишається один.
5. **Розподіл по проєктах.** `graph::call` передає інструмент до `scope::symbols`, `callers`,
   `impact`, `explore` або `outline`, які проходять по учасниках області й об'єднують відповіді
   в одному обмеженні.
6. **Вибір бекенду.** Для кожного проєкту `text_or` спершу перевіряє `text::applies`; інакше
   `door` вибирає за `backend`: `tags` відповідає з індексу; `lsp` і `auto` питають запис
   можливостей (`capability::ask`, перевіряється один раз) і викликають `lsp::Session`, а при
   відсутньому, мертвому сервері або порожній відповіді на ім'я, яке індекс знає, переходять на
   tags із позначкою `(tags; lsp: <reason>)`. `text` відповідає внутрішньопроцесним пошуком
   regex із позначкою `(text)`.
7. **Відповіді tags.** `symbol_tags` друкує визначення з вихідним кодом до `body_lines`;
   `callers_tags` групує рядки посилань за викликаючим визначенням (жодне ім'я не перелічує
   невикористані визначення); `impact_tags` обходить ребра викликів у ширину до `depth`, додає
   партнерів зі спільних змін git і, понад `impact_tokens`, групує файли за `blast.rs`;
   `explore_tags` зіставляє ідентифікатори запиту з визначеннями, додає ланцюжки викликів між
   ними і по одному рядку впливу для кожного; `outline` повторно використовує карту з `read`.
8. **Обмеження.** `cap_kind` архівує текст понад `max_tokens`, друкує початок плюс
   `N more, expand <id>` і записує `Measurement` лише тоді, коли відповідь було скорочено.
   Збережений HEAD, що відрізняється від поточного, додає префікс `index <old> head <new>`.
9. **Інші читачі.** `graph_diff` (`diff::call`) порівнює символи з посиланнями git, прочитаними з
   об'єктної бази git; `graph_export` (`export::call`) видає редагований JSON `rtok.graph.v1`
   (CLI може намалювати його через `draw.rs`); `review` і `affected` ранжують diff git; `drill`,
   `diff::page` та кадри експорту живлять сторінки графа у вебі та TUI.
10. **Карта репозиторію.** На SessionStart, коли `map_tokens > 0`, `repo_map` пропонує
    `rank::map` (або найчастіше посилані імена) як `Injection` пріоритету 1, який бюджетує `inject`.

## Де це живе

| Блок | Код |
| --- | --- |
| Плагін `GraphPlugin`, `mcp_tools`, `post_tool`, `session_start` | `src/plugins/graph/mod.rs` |
| Маршрутизація MCP, `events::Call`, `health::notice` | `src/mcp.rs` (`invoke`), `src/plugins/graph/events.rs`, `health.rs` |
| `rtok graph` | `src/cli/graph.rs` (`GraphCmd`) |
| `index_for`, `index::run`, `run_changed`, `ensure` | `src/plugins/graph/mod.rs`, `src/plugins/graph/index.rs` |
| `walk::Matcher` | `src/plugins/graph/walk.rs` |
| `rank::refresh`, `rank::map` | `src/plugins/graph/rank.rs`, `cochange.rs` |
| `watch::run_scope` | `src/plugins/graph/watch.rs` |
| `scope::*` | `src/plugins/graph/scope.rs` (`resolve`, `symbols`, `callers`, `impact`, `explore`) |
| Вибір бекенду | `src/plugins/graph/mod.rs` (`text_or`, `lsp_or_tags`, `door`), `capability.rs`, `lsp.rs`, `text.rs` |
| Бекенди tags | `src/plugins/graph/mod.rs` (`symbol_tags`, `callers_tags`, `impact_tags`, `explore_tags`), `blast.rs` |
| `cap_kind` | `src/plugins/graph/mod.rs` |
| diff, export, review | `diff.rs`, `export.rs`, `draw.rs`, `review.rs`, `drill.rs` |
| Сховище символів | `crates/rtok-store/src/symbols.rs`, `symbol_graph.rs` (таблиці `symbols`, `symbol_stale`, `extractor`, `file_rank`, `kv`, `graph_events`) |

## Інваріанти

- Лише нативно: індекс будується тут із тегів tree-sitter; жоден зовнішній інструмент графа не
  запускається і не підключається.
- Індексація ніколи не виконується на шляху хука; PostToolUse(Edit|Write) лише позначає файл
  застарілим, а сховище належить одному писачеві (спостерігач є потоком у `rtok mcp`).
- Кожна відповідь обмежена `plugins.graph.max_tokens` і несе id архіву, коли її обрізано;
  рядок `Measurement` існує лише тоді, коли відповідь було скорочено.
- Рядки `symbols` прив'язані до канонічного кореня, тож одне сховище містить багато репозиторіїв.
- Бекенд, який не може відповісти, відкрито переходить на індекс tags, а відповідь вказує, який
  бекенд відповів.
- Ключі конфігурації: `[plugins.graph] enabled`, `max_tokens`, `body_lines`, `impact_tokens`,
  `map_tokens`, `map_rank`, `auto_index`, `backend`, `watch`.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [Потік MCP](../mcp-flow.md) · [плагін read](read.md) · [плагін guard](guard.md) · [плагін inject](inject.md)
