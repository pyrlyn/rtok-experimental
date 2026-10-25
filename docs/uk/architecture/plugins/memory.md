---
lang: uk
---

# Плагін memory

![Нотатки надходять через mem_save, префікс remember, опційний екстрактор або контрольну точку, проходять шлюз редагування та допуску в сховище notes у SQLite з FTS5 і посиланнями на файли, і повертаються як обмежений індекс заголовків з хуків SessionStart, UserPromptSubmit і PreCompact або як результати інструментів mem_search, mem_pack і mem_get](../../../architecture/plugins/memory.svg)

Джерело: [`memory.d2`](../../../architecture/plugins/memory.d2). Документація плагіна: [README](../../../../src/plugins/memory/README.md). Індекс: [усі діаграми](../README.md).

`memory` є локальним сховищем нотаток без викликів моделі. Нотатки надходять від агента
(`mem_save`), від людини (промпти `remember:`), від механічного спостерігача за викликами
інструментів або від опційного екстрактора, який запускає програму, названу користувачем.
Відновлення вставляє лише індекс заголовків, і кожен запис коштує певну кількість токенів у
тілі, тож модель платить за тіло лише тоді, коли його просить (`mem_get`, `mem_pack`). Нічого
ніколи не видаляється: нотатки виводяться з ужитку, замінюються або закріплюються.

## Покроково

1. **Збереження.** Інструмент MCP `mem_save` викликає `mem_save_gated`. `strip_private` замінює
   кожен проміжок `<private>`...`</private>` на `[REDACTED]`; порожній заголовок або тіло
   відхиляються.
2. **Шлюз допуску.** Коли `[plugins.memory] admit` увімкнено, `admit::judge` відхиляє тіло
   коротше за `admit_min_chars` або таке, що на `admit_similarity` відсотків схоже за триграмами
   на збережену нотатку проєкту, і повертає `{"refused": "too_short"|"duplicate", "id": <stored>}`.
   Збереження з наявною парою `(project, kind, title)` є ревізією і завжди проходить.
3. **Upsert і посилання.** `save_scoped` викликає `upsert_note`: заголовок є ключем теми, тож
   повторне збереження оновлює рядок (старий заголовок і тіло переходять до `note_versions`) і
   знімає надгробок. Індекс `notes_fts` підтримується тригерами. `files::link_note` записує в
   `note_files` наявні відносно кореня файли, які згадує тіло, а `upsert_note_embedding` оновлює
   рядок вектора для опційних налаштувань `embed`.
4. **Інші записувачі.** Перший рядок промпту, що починається з `remember:`, зберігається як вид
   `user` через `remember_save` (з редагуванням, без шлюзу). `rtok memory extract` запускає
   `extract::run`, який передає відредагований хвіст транскрипта до `[plugins.memory.extract] command`
   і зберігає кожного кандидата як звичайну нотатку через той самий шлюзований збережувач; для
   цього потрібне `extract.enabled`, і жоден хук його не викликає. `checkpoint.rs` плагіна inject
   оновлює нотатку контрольної точки PreCompact і SessionEnd.
5. **Спостереження.** На PostToolUse `observe::capture` пропускає інструменти `mem_*`, будує
   типізований заголовок, очищену оповідь завдовжки до 400 символів і файли з вхідних даних
   інструмента, а потім вставляє один рядок у `observations` (і `observation_files`), дедуплікуючи
   за хешем сесії, інструмента і оповіді. Сирий вивід інструмента не копіюється.
6. **Відновлення через хук.** SessionStart запускає `recall`: `list_note_titles` для проєкту
   (закріплені першими), потім `render_title_index`, який тримає індекс у межах `recall_tokens`,
   відкидаючи найновіші записи. UserPromptSubmit запускає `prompt_recall` над першими 24 словами:
   `search_notes` (FTS або збережена гібридна ланка, коли ввімкнено `embed.enabled` і `embed.hybrid`),
   `files::recall_hits` зводить нотатки, прив'язані до файлів, які сесія читала (`read_cache`),
   або які назвав промпт, і після цього йде щонайбільше два рядки `obs`. PreCompact додає до
   контексту хоста `compact_context` (заголовки плюс спостереження цієї сесії), а SubagentStart
   додає `handoff::build_brief`, коли ввімкнено `spawn_brief`.
7. **Відновлення через інструмент.** `mem_search` є FTS5 bm25, ранжованим повторно за
   використанням, віком, закріпленням і проєктом. `mem_pack` пакує те саме ранжування в бюджет
   токенів ярус за ярусом (анотація, перший абзац, тіло), пропускаючи ярус, який не вміщується.
   `mem_get` повертає тіло (виведена нотатка містить рядок `retired <ts>`) і враховує використання.
   `mem_update` виводить з ужитку, замінює або закріплює.
8. **Кожен шлях вимірюється.** `recall`, `prompt_recall`, `pre_compact`, `mem_pack`, `mem_get`,
   `observe` і `brief` записують рядки `Measurement`, у яких сторона «до» є тілами, які замінив
   індекс.

## Де це живе

| Блок | Код |
| --- | --- |
| Поверхні | `src/mcp.rs` (`mem_save`, `mem_pack`, `mem_get`, `mem_update`), `src/hooks/mod.rs` (PreCompact), `src/cli/memory.rs` |
| strip_private | `src/plugins/memory/mod.rs` (`strip_private`, `redact_private`) |
| admit::judge | `src/plugins/memory/admit.rs` (`judge`, `Verdict`) |
| upsert_note, шлях збереження | `src/plugins/memory/mod.rs` (`mem_save_gated`, `save_scoped`) |
| files::link_note | `src/plugins/memory/files.rs` (`link_note`, `recall_hits`, `existing_rels`) |
| префікс remember: | `src/plugins/memory/mod.rs` (`remember_save`) |
| extract::run | `src/plugins/memory/extract.rs` (`run`, `run_configured`, `CommandExtractor`) |
| observe::capture | `src/plugins/memory/observe.rs` (`capture`, `synthesize`), `scrub.rs` |
| Контрольна точка | `src/plugins/checkpoint.rs` (`save`, `write`), використовується в `src/plugins/inject/` |
| recall, prompt_recall | `src/plugins/memory/mod.rs` (`recall`, `prompt_recall`, `render_title_index`, `compact_context`) |
| build_brief | `src/plugins/memory/handoff.rs` (`build_brief`, `handoff`) |
| mem_search, mem_pack, mem_get, mem_update | `src/plugins/memory/mod.rs`, `src/plugins/memory/pack.rs` (`pack_notes`) |
| import, export, sync | `src/plugins/memory/import.rs`, `export.rs`, `sync.rs` (лише CLI, на діаграмі не показано) |
| Таблиці | `crates/rtok-store/` (`notes`, `notes_fts`, `note_versions`, `note_embeddings`, `note_files`, `observations`, `observations_fts`, `observation_files`, `read_cache`, `measurements`) |
| Конфігурація | `crates/rtok-config/` (`[plugins.memory]`, `[plugins.memory.embed]`, `[plugins.memory.extract]`) |

## Інваріанти

- Без викликів моделі: нотатки надходять від агента, людини, механічного спостерігача або
  команди, названої користувачем.
- Життєвий цикл без втрат: виведення з ужитку, заміна і закріплення, але ніколи не видалення;
  виведене з ужитку тіло лишається доступним через `mem_get`.
- Проміжки `<private>` редагуються до того, як що-небудь зберігається, а оповіді спостережень
  очищаються до того, як їх обрізають.
- Один рядок на `(project, kind, title)`; upsert з тим самим тілом не записує версію.
- Відновлення вставляє лише заголовки та id в межах `recall_tokens`, стабільні за байтами для
  незмінного сховища; лише `mem_pack` може повернути абзац або тіло, і він ніколи не запускається
  через хук.
- Відмовостійкість: помилка сховища або пошуку залишає вивід хука без змін, і жодного мережевого
  виклику на шляху хука не робиться.
- Немає рядка `Measurement`, немає заяви про економію; лічильники промптів контрольної точки
  йдуть у стан плагіна, а не в облік.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [Потік хуків](../hook-flow.md) ·
[Потік MCP](../mcp-flow.md) · [Сховище](../store.md) · [inject](inject.md) · [read](read.md)
