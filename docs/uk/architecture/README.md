---
lang: uk
---

# Діаграми архітектури

Кожна частина rtok має [D2](https://d2lang.com)-діаграму тут і сторінку, яка її покроково описує, називаючи код, за яким стоїть кожен блок. Почніть з огляду, потім простежте запит через поверхню, яка вас цікавить, і відкрийте плагін.

[`architecture.md`](../../../architecture.md) викладає принципи й рішення прозою; ці сторінки показують механізм.

## Система

| Діаграма | Показує |
| --- | --- |
| [Огляд](overview.md) | Агенти, три поверхні, реєстр, плагіни, сховище та поверхні оператора |
| [Шари](layers.md) | Модульний DAG, який перевіряє `tests/module_graph.rs` |
| [Крейти](crates.md) | Робочий простір Cargo: бінарники, бібліотека і `crates/` |
| [Контракт плагіна](plugin-contract.md) | `Plugin`, `Ctx`, трейти можливостей, вибір за фічею та конфігурацією |
| [Сховище](store.md) | Таблиці SQLite за доменами, архів на диску і хто що читає |

## Потоки запитів

| Діаграма | Показує |
| --- | --- |
| [Потік хуків](hook-flow.md) | `rtok hook <event>`: захист, адаптація, розподіл за подією, злиття, fail open |
| [Потік MCP](mcp-flow.md) | `rtok mcp`: stdio, HTTP і обгортка; перелік, перевірка, виклик і запис виклику інструмента |
| [Потік проксі](proxy-flow.md) | `rtok proxy`: смуги, перевірка циклів і місткості, формування, пересилання та розгалуження |

## Плагіни

| Плагін | Діаграма |
| --- | --- |
| archive | [archive](plugins/archive.md) |
| checkpoint | [checkpoint](plugins/checkpoint.md) |
| cmd | [cmd](plugins/cmd.md) |
| compress | [compress](plugins/compress.md) |
| docs | [docs](plugins/docs.md) |
| graph | [graph](plugins/graph.md) |
| guard | [guard](plugins/guard.md) |
| inject | [inject](plugins/inject.md) |
| json_tree | [json_tree](plugins/json_tree.md) |
| measure | [measure](plugins/measure.md) |
| memory | [memory](plugins/memory.md) |
| proxy | [proxy](plugins/proxy.md) |
| read | [read](plugins/read.md) |
| toon | [toon](plugins/toon.md) |
| wasm | [wasm](plugins/wasm.md) |

## Редагування діаграми

Джерелом є файл `.d2`; поруч із ним лежить `.svg`, який генерується і комітиться. SVG рендерить у процесі чистий Rust-бекенд [`d2-render`](https://crates.io/crates/d2-render), тож бінарник `d2` не потрібен.

1. Відредагуйте `<name>.d2` і його сторінку `<name>.md`.
2. Запустіть `just diagrams`. Він перерендерить кожен застарілий SVG і завершиться з помилкою, якщо джерело використовує конструкцію, яку нативний бекенд не малює (`sql_table`, `sequence_diagram`, мітки в Markdown, `near` на вкладених фігурах, імпорти, шари, …). Так закомічена картинка ніколи не втрачає частину джерела.
3. `cargo test -p rtok --test diagrams` (частина `just docs-check`) падає, коли SVG відсутній або старший за своє джерело, а також коли діаграма не має сторінки.

Тримайте спільну палітру на початку кожного файлу: синій для поверхонь, фіолетовий для рантайму, зелений для частин плагінів, бурштиновий для даних, рожевий для виводу оператора, сірий для речей поза rtok. Нова сторінка також отримує переклад у `docs/ru/` і `docs/uk/`.
