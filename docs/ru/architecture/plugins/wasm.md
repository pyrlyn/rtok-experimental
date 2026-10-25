---
lang: ru
---

# Плагин wasm

![Хост с поддержкой функции читает файлы .wasm из каталога плагинов, запускает каждый экспорт манифеста под бюджетом fuel, отвергает поверхности хуков и регистрирует остальные как плагины, чьи вызовы MCP накапливают измерения для записи хостом](../../../architecture/plugins/wasm.svg)

Источник: [`wasm.d2`](../../../architecture/plugins/wasm.d2). Документация плагина: [`src/plugins/wasm.rs`](../../../../src/plugins/wasm.rs). Оглавление: [все диаграммы](../README.md).

Хост WASM позволяет третьей стороне поставлять плагин как файл `.wasm` вместо крейта Rust в этом репозитории. Он компилируется только с функцией Cargo `wasm-host` (на основе `wasmi`) и загружается только при `[plugins.wasm] enabled = true`. Это не плагин из каталога: он не перечислен в `plugins::all()` и не имеет собственного манифеста.

## Пошагово

1. **Реестр запрашивает внешние плагины.** `Registry::from_plugins` сначала строит нативный список через `config.plugin_enabled`, затем (функция `wasm-host`) вызывает `wasm::append`. Он возвращает список без изменений, когда `[plugins.wasm] enabled` равно false или каталог нельзя прочитать.
2. **Загружается каждый `*.wasm`.** `append` сканирует `[plugins.wasm] dir` (по умолчанию `~/.rtok/plugins`, один уровень) и вызывает `WasmPlugin::load`. Движок создаётся с `consume_fuel(true)`; `Module::new` проверяет модуль.
3. **Гость получает три импорта.** `Linker` открывает только `env.rtok_estimate`, `env.rtok_record_measurement` и `env.rtok_log`. Измерение, чей `plugin` не является собственным id гостя, прерывает вызов ловушкой (trap); корректное измерение только накапливается, а записывает его хост позже.
4. **Манифест читается под бюджетом.** `store.set_fuel(FUEL)` выполняется до `instantiate_and_start` и снова до экспорта гостя `rtok_manifest`, который возвращает упакованный указатель и длину JSON `WasmManifest` (`id`, `surfaces`, `default_on`, `title`, `summary`, `saves_tokens`). Чтения памяти гостя проходят через `guest_slice`, который отвергает отрицательный или выходящий за границы указатель вместо паники.
5. **Проверка.** Манифест, который перечисляет поверхность `hook`, отвергается, как и манифест без поверхности из `mcp`, `proxy` и `cli`. Любая ошибка загрузки пишется в stderr и в журнал (`wasm` / `load`), и пропускается только этот плагин.
6. **Регистрация.** Корректный плагин становится `WasmPlugin` (реализация `impl Plugin`, хранящая `Mutex<WasmInner>`) и добавляется с `config.plugin_enabled(id, default_on)`. Его `dashboard_page` берётся из текста манифеста гостя.
7. **Инструменты MCP.** `mcp_tools` вызывает экспорт гостя `rtok_mcp_tools` и разбирает JSON-список `ToolDef`. `invoke_mcp` вызывает `rtok_on_mcp_tool` со сбросом fuel, переносит накопленные измерения в `Ctx::record` и возвращает текст гостя. Сейчас `invoke_mcp` вызывают только тесты: `src/mcp.rs` пропускает инструменты id вне `plugins::all()` при построении `tools/list`, а у `invoke` нет ветки для них.

## Где это находится

| Блок | Код |
| --- | --- |
| Registry::from_plugins | `src/plugins/mod.rs` (`Registry::from_plugins`) |
| wasm::append | `src/plugins/wasm.rs` (`append`) |
| WasmPlugin::load | `src/plugins/wasm.rs` (`WasmPlugin::load`, `load_with_fuel`, `FUEL`) |
| Импорты Linker | `src/plugins/wasm.rs` (`rtok_estimate`, `rtok_record_measurement`, `rtok_log`) |
| Манифест корректен? | `src/plugins/wasm.rs` (`WasmManifest`, проверки поверхностей в `load_with_fuel`) |
| WasmPlugin | `src/plugins/wasm.rs` (`WasmPlugin`, `WasmInner`, `WasmHost`) |
| mcp_tools, invoke_mcp | `src/plugins/wasm.rs` (`mcp_tools`, `invoke_mcp`, `guest_bytes`) |
| Фильтр tools/list в MCP | `src/mcp.rs` (проверка `builtin` перед `p.mcp_tools()`) |
| Конфигурация | `crates/rtok-config/` (`[plugins.wasm]` `enabled`, `dir`) |
| Флаг функции | `Cargo.toml` (`wasm-host = ["dep:wasmi"]`) |
| Демонстрационный гость | `crates/rtok-wasm-demo-guest/` |

## Инварианты

- Fail open: сломанный `.wasm` записывается в журнал и пропускается, но никогда не валит реестр.
- Недоверенный гость: каждый вызов выполняется под бюджетом fuel (`FUEL`), поэтому бесконечный цикл прерывается ловушкой, а не вешает `rtok mcp` (тест `endless_guest_runs_out_of_fuel`); указатели гостя проверяются на границы (тест `guest_slice_rejects_what_used_to_panic`).
- Без поверхности хука: плагин WASM не может объявить `hook`, поэтому ничто внешнее не стоит на пути хука в 10 мс.
- Гость не может записывать строки для другого id плагина, а строки записывает хост через `Ctx::record`, поэтому экономия по-прежнему требует строки `Measurement`.
- По умолчанию выключено: при выключенной функции `wasmi` не линкуется, а при `enabled = false` ничего не читается.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Крейты](../crates.md)
