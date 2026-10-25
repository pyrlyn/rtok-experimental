---
lang: uk
---

# Плагін read

![Агент або викликає інструменти MCP read, search і tree, які визначають шлях під захистом кореня і відповідають контуром, вказівником дедуплікації, diff або обмеженим тілом, або запускає хук PreToolUse для Read, який відхиляє великий нативний Read на користь rtok read](../../../architecture/plugins/read.svg)

Джерело: [`read.d2`](../../../architecture/plugins/read.d2). Документація плагіна: [README](../../../../src/plugins/read/README.md). Індекс: [усі діаграми](../README.md).

`read` замінює файлові інструменти хоста трьома інструментами MCP (`read`, `search`, `tree`),
вивід яких формують під токени: контури замість цілих файлів, однорядковий вказівник на файл,
який модель уже має, diff для змінених файлів і обрізання з id архіву для всього великого. Хук
PreToolUse спрямовує хост від нативного `Read` великого файла до цих інструментів. Кожне
скорочення є лосслес через `expand <id>`.

## Покроково

1. **Надходить виклик інструмента.** `rtok mcp` маршрутизує `read`, `search` і `tree` до
   `plugins::read::read`, `search::search` і `search::tree` (`src/mcp.rs`).
2. **Спершу працює охоронець кореня.** `resolve_with` нормалізує шлях (за символічними
   посиланнями), відмовляє у файлах облікових даних, коли до сервера можна дістатися через HTTP,
   і приймає лише шляхи під cwd, `[plugins.read] allow_paths` та (у `rtok mcp`) сусідні дерева
   робочих копій і кореневі елементи `roots/list` клієнта (`roots::dynamic`). Усе інше є
   помилкою, яка називає шлях.
3. **Режим вибирає вигляд.** `read_with` читає файл, потім вибирає за `mode` (за замовчуванням
   `[plugins.read] default_mode`): `full` або `lines` з `range` нумерує рядки; `map` і
   `signatures` викликають `outline::render` (теги tree-sitter, заголовки markdown або перші
   рядки з приміткою для невідомої мови); `stripped` відкидає вузли коментарів
   (`outline::stripped`); `density:X` викликає `density::render`, який лишає рядки з найрідшими
   словами; `diff` є `full` з примусовим шляхом дельти.
4. **Перевірка повторного читання.** Ключ має вигляд `cache::key(path, mode, range)`.
   `cache::hit` порівнює нові байти з архівною копією, названою в `read_cache`. Однакові байти
   повертають `unchanged id <8-char id>` (рядок `dedup`). Змінені байти повертають уніфікований
   diff плюс `previous <id>` і `expand <id>`, коли diff менший за `delta_max_ratio` від розміру
   файла (рядок `delta`).
5. **Дедуплікація в межах сесії.** Інакше `plugin::identical_result` шукає той самий sha256 у всій
   сесії і повертає вказівник замість тіла (рядок `dedup`). `cache::remember` тоді архівує байти і
   записує рядок `read_cache`.
6. **Обмеження.** `cap` або `cap_recording` обрізає тіло довше за `max_chars`, архівує повний
   текст і лишає `archived <id>` у місці обрізу. `Measurement` записується для `map`,
   `signatures`, `density` і `stripped` (сирий файл проти повернутого тексту), а також для
   `read_cap`, `search_cap` і `tree_cap`, коли відбулося обрізання; не більше одного рядка на виклик.
7. **Пошук і дерево.** `search` і `tree` обходять через `ignore::WalkBuilder` (враховуючи
   `.gitignore`, пропускаючи `.git` і зв'язані робочі копії) і повертають рядки
   `path:line: snippet` або `path size`, обмежені `search_max` / `tree_depth`, а потім через
   `cap_recording`.
8. **Хук радить.** На PreToolUse для `Read` `hook::pre_tool` нічого не повертає, якщо `advice`
   вимкнено або файл не більший за `native_max_bytes`. Нативний `Read` з `limit` у межах
   `range_max_lines` пропускається, бо `Edit` хоста потребує нативного `Read`. Якщо `Edit` або
   `Write` цього шляху є серед останніх п'яти рядків PostToolUse, виклик пропускається, або, за
   увімкненої `delta` і наявного кешованого читання, відхиляється на користь `read(mode=diff)`.
   Інакше він відхиляється з точною назвою інструмента хоста для `read` від rtok. Кожна відмова
   записує рядок витрат `read`/`deny`.
9. **Інвалідація кешу.** `Plugin::post_tool` викликає `cache::invalidate`, який очищає кешований
   ключ для зміненого шляху лише тоді, коли `delta` вимкнено; за увімкненої `delta` застарілий
   запис якраз є тим, відносно чого обчислюється diff.

## Де це живе

| Блок | Код |
| --- | --- |
| rtok hook, rtok mcp | `src/hooks/`, `src/mcp.rs` (`read_file`, `search_files`, `tree_files`) |
| Порада щодо Read | `src/plugins/read/hook.rs` (`pre_tool`, `recently_edited`, `deny`, `WINDOW_TOOL_CALLS`) |
| Охоронець кореня | `src/plugins/read/mod.rs` (`resolve_with`), `src/plugins/read/roots.rs` (`dynamic`, `client_roots`) |
| Перемикач режимів | `src/plugins/read/mod.rs` (`read_with`) |
| Контур | `src/plugins/read/outline.rs` (`render`, `stripped`, `tags`) |
| Щільність | `src/plugins/read/density.rs` (`parse`, `render`) |
| cache::hit, дельта | `src/plugins/read/cache.rs` (`hit`, `remember`, `invalidate`, `DEDUP_PREFIX`) |
| identical_result | `src/plugin.rs` (`identical_result`) |
| Обмеження | `src/plugins/read/mod.rs` (`cap`, `cap_recording`, `cap_impl`) |
| search / tree | `src/plugins/read/search.rs` (`search`, `tree`, `text_files`) |
| Маніфест | `src/plugins/read/mod.rs` (`ReadPlugin`: `Surface::Mcp`, `Surface::Hook`) |
| Конфігурація | `crates/rtok-config/` (`[plugins.read]`) |
| Таблиці | `crates/rtok-store/` (`read_cache`, `archive`, `measurements`) |

## Інваріанти

- Спершу охоронець кореня: жодного доступу до файлової системи за межами дозволених коренів;
  `allow_paths = [""]` не дає нічого.
- Лосслес: обмежений або дедуплікований результат завжди несе id, який розв'язує `expand`.
- Відповідь дедуплікації коротша за тіло, яке вона замінює; `identical_result` нічого не
  повертає, коли вказівник не був би коротшим.
- Хук працює за принципом fail open: помилка сховища, невідомий хост або файл, який не можна
  прочитати, пропускає `Read`, а нативний `Read` з діапазоном ніколи не відхиляється.
- Кожна відмова записується як витрата, а не як економія, тож `rtok stats --plugin read` віднімає її.
- Описи інструментів лишаються в межах їхнього оціненого бюджету токенів (тест у наборі T4.1).

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [Потік хуків](../hook-flow.md) ·
[Потік MCP](../mcp-flow.md) · [archive](archive.md) · [graph](graph.md)
