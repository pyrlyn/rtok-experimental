---
lang: uk
---

# Плагін inject

![inject: пропозиції хуків від кожного плагіна проходять через один бюджет токенів, відсортований за пріоритетом, перш ніж потрапити до моделі як додатковий контекст](../../../architecture/plugins/inject.svg)

Джерело: [`inject.d2`](../../../architecture/plugins/inject.d2). Документація плагіна: [README](../../../../src/plugins/inject/README.md). Індекс: [усі діаграми](../README.md).

`inject` є входом у контекст SessionStart, PostCompact, UserPromptSubmit і SubagentStart. Диспетчер
хуків збирає `Injection { plugin, text, priority }` з кожного ввімкненого плагіна, після чого
`inject::apply` виводить їх в межах одного бюджету, тож жодна частина не потрапляє до моделі
без бюджету. Плагін також додає власні пропозиції (контрольну точку після стиснення, передачу
попередньої сесії та налаштовані режими промптів) і записує контрольну точку на `PreCompact`.

## Покроково

1. **Спрацьовує подія хука.** `hooks::inject_event` у `src/hooks/mod.rs` викликає
   `session_start`, `prompt_submit` або `subagent_start` у кожному ввімкненому плагіні (подія
   `PostCompact` диспетчеризується як `session_start` з `source: "compact"`). Плагін, що впав,
   записується в журнал і пропускається.
2. **Долучаються додаткові пропозиції.** Диспетчер додає рядок id агента (SessionStart і
   SubagentStart), рядок прикріпленого завдання (SessionStart і PostCompact) та очікувані
   повідомлення (UserPromptSubmit); усі вони йдуть під тим самим бюджетом.
3. **`InjectPlugin::session_start` вибирає свій текст.** На `compact` він запитує у
   `checkpoint::offer` контрольну точку цієї сесії; на `startup`, коли ввімкнено
   `[plugins.memory] startup_recall`, запитує у `checkpoint::offer_session` передачу останньої
   сесії проєкту. Будь-який із цих текстів має пріоритет 9.
4. **Режими додаються.** `modes_text` бере `[plugins.inject] modes`, а якщо його немає, то
   `[setup] modes`. Кожна назва перетворюється на `<modes_dir>/<name>.md` або на вбудований режим
   (`terse` / `cave`, `yagni` / `pony`, `nudges`, вбудовані через `include_str!`), читається з
   обмеженим `read_mode` і підганяється під бюджет. Текст об'єднується в той самий `Injection`,
   чий пріоритет дорівнює 5, або 9, коли першим ішов текст контрольної точки чи передачі. Лише
   `session_start` додає текст режимів.
5. **`apply` забезпечує бюджет.** `inject::apply` сортує за пріоритетом (за рівності — за id
   плагіна), виводить кандидатів, поки `used < budget_tokens`, а для кандидата понад бюджет
   лишає префікс `fit_budget` плюс маркер `dropped:<plugin>:<est>`. Решта позначаються лише
   маркерами.
6. **Записується вимірювання.** `Measurement { kind: "inject" }` несе розміри запропонованих і
   виведених байтів та оцінки; `ref_id` містить перший маркер відкинутого. Якщо пропозицій немає
   взагалі, нічого не записується.
7. **PreCompact зберігає.** `InjectPlugin::pre_compact` викликає `checkpoint::save`, який читає
   транскрипт потоком і оновлює рядок `notes` виду `checkpoint:<session>`; SessionEnd записує
   рядок `session:<session>`, який `offer_session` читає пізніше.

## Де це живе

| Блок | Код |
| --- | --- |
| `hooks::inject_event` | `src/hooks/mod.rs` (`inject_event`) |
| `InjectPlugin::session_start`, `pre_compact` | `src/plugins/inject/mod.rs` (`impl Plugin for InjectPlugin`) |
| `modes_text`, `read_mode`, `builtin` | `src/plugins/inject/mod.rs` |
| `inject::apply` | `src/plugins/inject/mod.rs` (`apply`) |
| `checkpoint::offer`, `offer_session`, `save` | `src/plugins/checkpoint.rs` |
| Вбудовані режими | `modes/terse.md`, `modes/yagni.md`, `modes/nudges.md` |
| Ключ бюджету | `crates/rtok-config/src/lib.rs` (`plugins.inject.budget_tokens`, `modes_dir`) |

## Інваріанти

- Бюджет є жорсткою межею: `plugins.inject.budget_tokens` (за замовчуванням 800) обмежує те, що
  виводить `apply`, а відрізане позначається маркером `dropped:`. Тест:
  `three_500_budget_800_drops_one_and_is_byte_stable`.
- Вивід SessionStart стабільний за байтами для незмінного стану: у тексті, який пропонує
  `inject`, немає міток часу чи лічильників.
- Режими є даними у markdown-файлах, а не рядками Rust; відсутній або нечитабельний файл
  замінюється вбудованим, а невідома назва пропускається.
- `UserPromptSubmit` ніколи не несе тексту режимів (`session_start` є єдиним місцем, де він
  додається).
- `core.inject_budget_tokens` є видаленим псевдонімом; він приймається зі старих конфігурацій і
  переноситься до `plugins.inject.budget_tokens`.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [Потік хуків](../hook-flow.md) · [плагін memory](memory.md) · [плагін guard](guard.md)
