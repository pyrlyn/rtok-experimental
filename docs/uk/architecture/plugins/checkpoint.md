---
lang: uk
---

# Плагін checkpoint

![PreCompact і SessionEnd читають транскрипт і зберігають нотатку-контрольну точку; SessionStart читає нотатку назад як інжекцію в межах бюджету](../../../architecture/plugins/checkpoint.svg)

Джерело: [`checkpoint.d2`](../../../architecture/plugins/checkpoint.d2). Вихідний код модуля: [`checkpoint.rs`](../../../../src/plugins/checkpoint.rs). Індекс: [усі діаграми](../README.md).

`checkpoint` не є плагіном каталогу: у нього немає маніфесту і таблиці `[plugins.checkpoint]`.
Це модуль, який викликають `inject` і гілка `SessionEnd` диспетчера хуків. Перед тим як хост
стискає контекст або коли сесія завершується, модуль перетворює JSONL-транскрипт на коротку
нотатку: останні набрані промпти, торкнуті файли, останні помилки та id архіву, які досі
живі. Після стиснення або на наступному старті нотатка повертається як одна інжекція, щоб
модель не почала працювати наосліп.

## Покроково

1. **Збереження на PreCompact.** Диспетчер хуків викликає `pre_compact` кожного ввімкненого
   плагіна; `inject` викликає `checkpoint::save(transcript_path, cx)`, який записує нотатку
   виду `checkpoint:<session>`.
2. **Збереження на SessionEnd.** Гілка `SessionEnd` у `src/hooks/mod.rs` викликає
   `checkpoint::save_session` всередині `catch_unwind` і ігнорує результат. Вона використовує той
   самий екстрактор і рендер, вид `session:<session>`, прив'язаний до імені проєкту.
3. **Витяг.** `extract_path` читає транскрипт рядок за рядком, тож велика сесія ніколи не
   утримується в пам'яті. `worth_parsing` пропускає рядки, які не можуть містити шлях, помилку,
   набраний промпт або тіло скілу; тест порівнює відфільтрований і невідфільтрований прогони
   (`prefilter_is_a_true_superset_on_fixtures_and_a_large_transcript`). Результат є
   `Checkpoint`: останні 20 набраних промптів, шляхи файлів з `Action` (`Edited`, `Created`,
   `Deleted`, `Read`; спершу змінені файли), останні 8 рядків помилок (пропущені через
   `redact_private`) і тіла скілів, які вставив хост.
4. **Прикріплення id архіву.** `attach_ids` читає `session_live_archives` і додає id,
   починаючи з найновішого, доки `render()` ще вміщується в `[plugins.memory] checkpoint_tokens`;
   id, який не вміщується, відкидається, і цикл зупиняється.
5. **Збереження.** `write` викликає `upsert_note(project, kind, "compact", render())`, тож
   повторний PreCompact або SessionEnd тієї самої сесії замінює нотатку, а не додає нову. Він
   також зберігає кількість набраних і пропущених промптів як рядок ключ-значення стану плагіна;
   ці лічильники є сигналом якості, а не `Measurement`.
6. **Відновлення після стиснення.** На `SessionStart` з `source = "compact"`
   `inject::session_start` викликає `checkpoint::offer`, який читає останню нотатку цієї сесії.
7. **Відновлення на старті.** Коли `source = "startup"`, `[plugins.memory] enabled` і
   `startup_recall` увімкнені, `inject` викликає `checkpoint::offer_session`: найновіша нотатка
   `session:*` проєкту (із запасним варіантом за застарілим іменем для кожного checkout), плюс
   `Measurement` виду `handoff` під плагіном `memory`.
8. **Бюджетування інжекції.** `render_offer` знову запускає `redact_private` (старіші нотатки
   можуть містити приватний тег), обрізає текст через `fit_budget` до `checkpoint_tokens` і
   повертає `Injection` із `priority` 9. `inject` вкладає її в `additionalContext` хука.
9. **Побічне використання.** `last_paths(cx, root)` повертає файли останньої контрольної точки;
   карта репозиторію графа і код файлів пам'яті використовують її, щоб персоналізувати свій вивід.

## Де це живе

| Блок | Код |
| --- | --- |
| Підключення PreCompact / SessionStart | `src/plugins/inject/mod.rs` (`pre_compact`, `session_start`) |
| Підключення SessionEnd | `src/hooks/mod.rs` (гілка `"SessionEnd"`) |
| `save`, `save_session`, `write` | `src/plugins/checkpoint.rs` |
| `extract_path`, `worth_parsing` | `src/plugins/checkpoint.rs` (`extract_lines_with`) |
| `Checkpoint`, `Action`, `PathEntry` | `src/plugins/checkpoint.rs` |
| `attach_ids` | `src/plugins/checkpoint.rs` (`attach_ids`, `checkpoint_cap`) |
| `offer`, `offer_session`, `render_offer` | `src/plugins/checkpoint.rs` |
| `last_paths` | `src/plugins/checkpoint.rs`; виклики в `src/plugins/graph/mod.rs` і `src/plugins/memory/files.rs` |
| Таблиці | `crates/rtok-store/src/schema.rs` (`notes`, `archive_decisions`, `measurements`) |
| Конфігурація | `crates/rtok-config/src/lib.rs` (`Memory`: `checkpoint_tokens`, `startup_recall`) |

## Інваріанти

- Виклики хуків ніколи не падають через цей модуль: PreCompact ігнорує результат `save`, а
  SessionEnd загортає `save_session` у `catch_unwind`.
- Обмежений стан: екстрактор зберігає лише останні промпти й помилки та множину шляхів, тож
  пам'ять не зростає разом із транскриптом.
- Приватний текст редагується до збереження і ще раз перед відновленням.
- Відновлення обмежене `checkpoint_tokens`; той самий транскрипт рендериться в ідентичний за
  байтами текст, а тест перевіряє, що Claude, pi і OpenCode відновлюють однаковий текст
  (`pi_and_opencode_compact_restore_bytes_match_claude`).
- Єдине вимірювання: рядок `handoff` під час запуску; немає рядка, немає заяви про економію.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) ·
[inject](inject.md) · [memory](memory.md) · [graph](graph.md)
