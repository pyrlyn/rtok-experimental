---
lang: uk
---

# Плагін measure

![measure: транскрипти, рядки використання проксі та рядки Measurement живлять rtok stats, базові лінії, здоров'я кешу і rtok bench](../../../architecture/plugins/measure.svg)

Джерело: [`measure.d2`](../../../architecture/plugins/measure.d2). Документація плагіна: [README](../../../../src/plugins/measure/README.md). Індекс: [усі діаграми](../README.md).

`measure` є істинним еталоном, за яким оцінюють інші плагіни: нічого не випускається, поки
воно не покаже число до і після. Сама структура плагіна лише оголошує маніфест (поверхні `Cli`
і `Proxy`, увімкнено за замовчуванням) і сторінку панелі; основна робота міститься в `src/measure/`,
`src/bench.rs` і командах `rtok stats` / `rtok bench`, які до них звертаються. Він ніколи не
стоїть на шляху хука.

## Покроково

1. **Вхідні дані.** Три джерела його живлять: транскрипти агентів, які розбираються на льоту
   (JSONL Claude Code, події `token_count` Codex), рядки `usage`, які проксі записує для кожного
   запиту (`record_usage` у `src/proxy/mod.rs`, через `Store::insert_usage`), і рядки
   `measurements`, які інші плагіни записали через `Ctx::record`.
2. **Вибір вигляду.** `src/cli/stats.rs` спершу розгалужується за прапорцем: `--calibrate`
   друкує `calibrate_or_skip`; `--cache` рендерить `cache::report` (ходи, де створення кешу
   стрибнуло, а читання з кешу впало, з причиною, прочитаною з тіл `call_io`); `--plugin <id>`
   повертає рядки `measurements` цього плагіна через `model::plugin_stats`; будь-що інше будує
   повний звіт у `model::stats_report`.
3. **Розбір.** `jsonl::parse_path` читає один JSON-об'єкт на рядок. Некоректний рядок
   враховується в `Parsed::malformed` і не є фатальним; транскрипт, який не можна прочитати,
   враховується як один некоректний запис.
4. **Збір.** `stats::collect` обходить транскрипти в порядку шляхів, пропускає файли субагентів
   (`subagents::is_subagent`, їх зараховують батьківській сесії) і згортає сесії у `Report`:
   розміри результатів за інструментами, родини команд Bash (`bash_family`), групи MCP і
   контекстні токени за ходами.
5. **Прикріплення даних сховища.** `model::stats_report` відкриває сховище (для звіту воно
   необов'язкове) і викликає `attach_api`, `attach_lanes`, `attach_bash_cmd`,
   `attach_checkpoint_notes` та `attach_costs`, коли задано `--price`; `attach_codex` додає
   запити Codex.
6. **Базові лінії.** `--save-baseline <name>` записує JSON звіту в
   `<home>/measurements/<name>.json` (`baseline::save`, який відхиляє імена з роздільниками
   шляхів); `--compare <name>` рендерить різницю через `baseline::compare`. Інакше звіт друкується
   як таблиця або, з `--json`, як JSON.
7. **Бенчмарк.** `rtok bench` викликає `bench::run`: типовий набір запускає шість завдань за
   `[bench.configs]`, `--suite graph` порівнює з сервером MCP rtok і без нього, а `--suite p28`
   запускає набір p28. Без `RTOK_BENCH_LIVE` повертається нуль рядків замість запуску агента.
   Результати потрапляють у `results/<config>.json` поруч із файлом завдань.

## Де це живе

| Блок | Код |
| --- | --- |
| Маніфест плагіна measure | `src/plugins/measure/mod.rs` (`MeasurePlugin`) |
| `jsonl::parse_path` | `src/measure/jsonl.rs` (`Parsed`, `parse_jsonl`, `parse_dir`) |
| `stats::collect` | `src/measure/stats.rs` (`collect`, `Report`, `bash_family`) |
| Кроки прикріплення | `src/measure/stats.rs` (`attach_api`, `attach_lanes`, `attach_bash_cmd`, `attach_codex`, `attach_costs`) |
| `cache::report` | `src/measure/cache.rs` (`analyse`, `report`, `SessionHealth`) |
| `baseline::save / compare` | `src/measure/baseline.rs` |
| `model::stats_report` | `src/model/mod.rs` (`stats_report`, `plugin_stats`, `cache_health`) |
| `rtok stats` | `src/cli/stats.rs` |
| `bench::run` | `src/bench.rs`, `src/bench/p28.rs`, `src/cli/bench.rs` |
| Захоплення використання проксі | `src/proxy/mod.rs` (`record_usage`) |
| Читач Codex | `src/measure/codex.rs` (`requests`, `collect`) |
| `Ctx::record` | `src/plugin.rs` (`Runtime::record`) |

## Інваріанти

- Економією вважаються лише рядки `Measurement`; числа з транскриптів є оцінками, а єдиними
  реальними лічильниками провайдера є рядки `usage` проксі.
- Некоректний рядок транскрипта пропускається та враховується; він ніколи не перериває звіт.
- Транскрипти та читання Codex ніколи не записуються в сховище, тож повторне читання ідемпотентне.
- `--calibrate` є єдиним викликом мережевого типу і без ключа API виводить `skipped`.
- Сховище, яке не відкривається, позбавляє `rtok stats` лише таблиці API і нічого більше, окрім
  `--plugin` і `--cache`, яким сховище потрібне.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [плагін proxy](proxy.md) · [плагін archive](archive.md)
