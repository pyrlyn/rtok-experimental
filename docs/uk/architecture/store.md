---
lang: uk
---

# Сховище

![Сховище SQLite: поверхні, плагіни та код координації записують через rtok-store в одну базу WAL, згруповану за доменами; сирі payload-и спливають у archive_dir, а expand, stats, модель оператора і експорт OTLP читають її](../../architecture/store.svg)

Джерело: [`store.d2`](../../architecture/store.d2). Індекс: [усі діаграми](README.md).

rtok тримає один файл SQLite (D8). Хуки — короткоживучі процеси, тож кожен виклик відкриває файл, записує свої рядки й завершується; режим WAL дозволяє багатьом одночасним записувачам (кожному хуку, серверу MCP, проксі) ділити його. Цей файл — доказова база продукту: економія або є рядком `measurements`, або її немає.

## Покроково

1. **Один власник.** `rtok-store` — єдиний крейт, що називає Diesel. `Store::open_with` відкриває файл із обмеженим очікуванням блокування, перемикає його в WAL, застосовує вбудовані міграції (`crates/rtok-store/migrations/NNNN_<slug>/`, записані в `schema_migrations`) і робить файл доступним лише власнику. Крейт нічого не друкує; проблеми з обслуговуванням повертаються як `StoreWarning`.
2. **Рядки ідентичності** (`hosts`, `providers`, `models`, `sessions`) створюються або оновлюються (upsert) і посилаються кожним викликом. `hook_sessions` зберігає поля сесії хука один раз для кожного окремого набору, тож `call_io` не повторює їх.
3. **Журнал.** Кожна поверхня записує один рядок `calls` на кожну подію, виклик MCP або запит до API, а байти запиту й відповіді — у `call_io` (вбудовано до `core.call_io_inline_bytes`, інакше спливають у `archive_dir`). `tokens` тримає значення до/після/провайдера, `measurements` — економію плагінів (рядок хука має `once_key`, тож повторна доставка враховується один раз), `usage` — справжнє використання від провайдера через проксі, `logs` — рядки логу.
4. **Архів.** Усе скорочене зберігається повністю: рядок `archive` містить розмір, sha256 і шлях, а байти лежать у `archive_dir/<id>`. `rtok expand <id>` (і інструмент MCP `expand`) читають їх назад. `read_cache` дозволяє `read` пропускати файл, який сесія вже бачила.
5. **Пам'ять.** `notes` з його індексом FTS5 `notes_fts`, `note_files`, версії, необов'язкові ембедінги та `observations`, які витягує плагін memory.
6. **Граф і документація.** Індекс символів плагіна `graph` (`symbols`, `symbol_stale`, `file_rank`, `extractor`), його журнал подій і реєстр проєктів; `doc_crates` / `doc_items` плагіна `docs`.
7. **Координація.** Рядки агентів, повідомлення між агентами, закріплення worktree і завдань та лічильники, які читають і записують інструменти `worktree_*`, `agent_*` і `task_*`.
8. **Читачі.** `rtok stats`, `report` і `doctor` агрегують журнал; `src/model` будує модель оператора D23, яку рендерять `rtok tui` і `rtok web`; експортер OTLP передає журнал повз свою позначку `otel_export`. Очищення за строком зберігання та vacuum запускаються з довготривалих поверхонь (`spawn_retention`).

## Де це живе

| Блок | Код |
| --- | --- |
| Відкриття, WAL, права доступу | `crates/rtok-store/src/handle.rs` |
| Схема і міграції | `crates/rtok-store/src/schema.rs`, `crates/rtok-store/migrations/` |
| Виклики, токени, вимірювання | `crates/rtok-store/src/call_log.rs`, `measure_rows.rs` |
| Архів | `crates/rtok-store/src/archive_rows.rs`, `archive_summaries.rs`, `src/expand.rs` |
| Нотатки і спостереження | `crates/rtok-store/src/note_store.rs`, `note_files.rs`, `observations.rs` |
| Символи | `crates/rtok-store/src/symbols.rs`, `symbol_graph.rs` |
| Координація | `crates/rtok-store/src/agents.rs`, `messages.rs`, `worktree_claims.rs`, `task_claims.rs` |
| Очищення за строком | `crates/rtok-store/src/retention.rs` |

## Інваріанти

- Доступ до бази йде через Diesel; сирий SQL з'являється лише там, де Diesel не може його виразити (міграції, FTS5), всередині крейта сховища.
- Застосовану міграцію ніколи не редагують; зміна — це наступна тека міграції.
- Заблоковане сховище ніколи не валить хук: хук пропускає виклик далі і відкладає те, що мусить.

## Пов'язане

[Огляд](overview.md) · [Контракт плагіна](plugin-contract.md) · [Потік хуків](hook-flow.md) · [Потік проксі](proxy-flow.md) · [archive](plugins/archive.md) · [memory](plugins/memory.md) · [graph](plugins/graph.md) · [measure](plugins/measure.md)
