---
lang: ru
---

# Версии и обновления плагинов

Как rtok версионирует деревья плагинов в `plugins/<host>/`, записывает, что он установил, и
решает, что `rtok agents update` делает с установленным плагином.

**Охват на сегодня:** к решению по версии подключён только Claude Code. Codex, Copilot и Gemini
(и все остальные хосты) несут файл версии, но сохраняют прежнее поведение `agents update`;
они последуют позже.

Каждый пример ниже — реальный запуск rtok 0.10.0 с одноразовым `HOME` и заглушкой
`claude` первой в `PATH` (настоящий Claude Code не участвует). `--cli --no-restart` ограничивают запуск
CLI Claude Code; каждый отчёт сокращён до заголовка, строк плагина и ошибок.

## Зачем

Claude Code кеширует плагин по `version` в его манифесте. До rtok 0.10.0 манифест каждого плагина
указывал `0.0.1`, поэтому новая сборка rtok поставляла плагин, который у Claude уже был «в этой
версии», и он его никогда не подхватывал. Сам rtok не хранил записи о том, какая сборка плагина установлена и
откуда она взялась, поэтому `agents update` мог только каждый раз переустанавливать или доверять хосту.

Плагин попадает на машину из одного из трёх источников, и схема охватывает их все:

- **GitHub**: хост устанавливает из репозитория (`claude plugin marketplace add
  pyrlyn/rtok`).
- **Local**: хост устанавливает из дерева плагина на диске (`claude plugin marketplace add
  <path>/plugins/claude`) — для разработки и офлайн-установок.
- **Marketplace**: запись каталога хоста для закоммиченного `.claude-plugin/marketplace.json`.

## Файл версии

Каждое версионированное дерево плагина содержит закоммиченный файл `plugins/<host>/.rtok-plugin-version` — одну строку
JSON:

```text
$ cat plugins/claude/.rtok-plugin-version
{"schema":1,"plugin":"claude","version":"0.10.0"}
```

| Поле | Значение |
| --- | --- |
| `schema` | Версия формата. rtok читает только `1` и отклоняет любое другое значение с ошибкой, называющей файл. |
| `plugin` | Имя каталога хоста (`claude`, `codex`, …). |
| `version` | SemVer; равна версии в `Cargo.toml` этого коммита. Некорректная версия — ошибка, называющая файл. |
| `source` | Необязательное поле, зарезервировано для локальной установки (`github`, `local`, `marketplace`). В закоммиченных файлах его никогда нет. |

Хосты с файлом: `claude`, `codebuddy`, `codex`, `copilot`, `cursor`, `devin`, `gemini`, `grok`, `kimi`,
`opencode`, `pi`, `zcode`. У `cline` нет собственного манифеста, а в манифесте `antigravity` нет
поля версии, поэтому ни у одного из них файла нет. Та же версия записывается в поле `version` каждого манифеста
(`plugins/claude/.claude-plugin/plugin.json` и остальные), поэтому хост, который кеширует по
версии манифеста, видит каждый релиз как новый.

Эти файлы пишет только релиз ([Выпуск релиза](#выпуск-релиза)). Файл лежит в корне плагина,
поэтому любой источник копирует его вместе с плагином в установку хоста (для Claude — в его кеш
плагинов в `~/.claude/plugins/cache/rtok/`). rtok пока не читает файл обратно из
установленной копии и не записывает в неё копию с `+g<sha>`: для локальной сборки строка сборки хранится
только в квитанции.

## Квитанция `plugins.json`

rtok хранит по одной строке на хост с тем, что он установил (сегодня строку пишет только `claude`):

| ОС | Путь |
| --- | --- |
| macOS | `~/Library/Application Support/rtok/plugins.json` |
| Linux | `$XDG_STATE_HOME/rtok/plugins.json`, иначе `~/.local/state/rtok/plugins.json` |
| Windows | `%LOCALAPPDATA%\rtok\plugins.json`, иначе `%USERPROFILE%\AppData\Local\rtok\plugins.json` |

После обновления из GitHub (домашний путь сокращён до `~`):

```json
{
  "claude": {
    "source": "github",
    "ref": "v0.10.0",
    "marketplace": "rtok",
    "path": "~/.claude/plugins/installed_plugins.json",
    "version": "0.10.0",
    "installed_at": "2026-09-27T20:17:52Z"
  }
}
```

| Поле | Значение |
| --- | --- |
| `source` | `github`, `local` или `marketplace`. |
| `ref` | `v<version>` для GitHub и marketplace; путь к локальному дереву плагина для локальной установки. |
| `marketplace` | `rtok` для GitHub и marketplace; отсутствует для local. |
| `path` | Запись хоста, в которой видна установка (для Claude — `installed_plugins.json`). |
| `version` | Установленная версия; локальная установка добавляет `+g<sha>` или `+g<sha>.dirty`. |
| `installed_at` | UTC, RFC 3339, с точностью до секунды. |

Строка локальной установки из checkout с незакоммиченными изменениями (путь checkout сокращён):

```json
    "source": "local",
    "ref": "<checkout>/plugins/claude",
    "version": "0.10.0+g84a4b607.dirty",
```

Строка записывается после успешной установки, переустановки или обновления на месте и не трогается при
`--dry-run` и при любом сбое `claude`, который ничего не удалил. Она удаляется, когда переустановка
уже удалила старый плагин, а установка затем не удалась ([Решение](#решение)).
Отсутствующий файл читается как «строк нет».

## Источники

Для Claude `agents update` выбирает источник в таком порядке: `--source`, иначе `source` из строки квитанции,
иначе GitHub.

| Источник | На что указывает marketplace `rtok` | Доступная версия |
| --- | --- | --- |
| GitHub | `pyrlyn/rtok` | собственная версия запущенного rtok |
| Marketplace | `pyrlyn/rtok` | собственная версия запущенного rtok |
| Local | дерево плагина, которое находит rtok (`plugins/claude` рядом с бинарником, в префиксе `share/rtok`, в хранилище ketch или в checkout исходников) | `.rtok-plugin-version` этого дерева плюс `+g<sha>[.dirty]` из `git describe --always --dirty` |

GitHub и marketplace не требуют сетевого вызова: релиз держит каждый файл версии равным
`Cargo.toml`, поэтому тег `v<version>`, соответствующий бинарнику, несёт ровно эту версию. Отличаться от бинарника может
только локальное дерево, поэтому только оно читается с диска. Сегодня GitHub и marketplace
различаются лишь записанным источником.

Установленная версия берётся из строки квитанции, иначе из собственной записи Claude
(`~/.claude/plugins/installed_plugins.json`, `plugins["rtok@rtok"][0].version`), если она в формате
SemVer, иначе `0.0.0`. Считается, что у Claude плагин есть, когда этот файл содержит `rtok@rtok`.

Запись `rtok` в `~/.claude/plugins/known_marketplaces.json` сверяется с выбранным
источником: `{"source":"github","repo":"pyrlyn/rtok"}` для GitHub и marketplace,
`{"source":"directory","path":"<local tree>"}` для local. Любое другое значение (например, путь к хранилищу ketch
до 0.10) считается устаревшим и вызывает переустановку, которая перенаправляет запись.

Репозиторий переехал из аккаунта `listepo` в организацию `pyrlyn`. Marketplace,
добавленный до переезда, по-прежнему записывает `{"source":"github","repo":"listepo/rtok"}` для Claude или
`source = "https://github.com/listepo/rtok.git"` в `[marketplaces.rtok]` для Codex. Обе записи
по правилу выше устарели. Следующий `rtok agents install` или `agents update` удаляет
marketplace `rtok`, добавляет его снова из `pyrlyn/rtok` и один раз переустанавливает `rtok@rtok`. Имена
marketplace и плагина остаются `rtok`, поэтому `rtok@rtok` по-прежнему означает тот же плагин. Чтобы перейти
вручную: `claude plugin marketplace remove rtok && claude plugin marketplace add
pyrlyn/rtok && claude plugin install rtok@rtok` (Codex: то же с `codex plugin`, а для
последнего шага — `plugin add`).

## Решение

Одна чистая функция (`decide` в `src/agents/plugin_version.rs`) сравнивает установленную и
доступную версии по старшинству SemVer, которое игнорирует метаданные сборки, а затем отдельно сравнивает метаданные
сборки. Строки проверяются сверху вниз; побеждает первое совпадение.

| Случай | Результат | Вызовы `claude` |
| --- | --- | --- |
| Плагин не установлен | установка | `plugin marketplace add` (если запись не актуальна), `plugin install rtok@rtok` |
| `--force` | переустановка | `plugin uninstall`, затем как выше |
| Источник изменился или запись marketplace устарела | переустановка | `plugin uninstall`, `plugin marketplace remove rtok` и `add` (если запись не актуальна), `plugin install` |
| Доступная новее | обновление на месте | `plugin marketplace update rtok`, `plugin update rtok@rtok` |
| Та же базовая версия, другие метаданные сборки | обновление на месте | как выше |
| Устаревшая установка: квитанции нет, запись хоста `0.0.1` или не SemVer (`0.0.0`) | обновление на месте (старше любого релиза) | как выше |
| Совпадают версия и метаданные сборки | пропуск | нет |
| Доступная старше | пропуск с предупреждением | нет |

Если обновление на месте не удалось, rtok переходит к переустановке и добавляет ошибку к выводу.

Устаревшая установка обновляется один раз и записывает квитанцию; повторный запуск не вызывает ни одной команды `claude`:

```text
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
~ plugin rtok@rtok updated to 0.10.0
$ rtok agents update claude --cli --no-restart
CLI: Claude Code — already current
```

`--dry-run` показывает решение и ничего не записывает. Обновление или пропуск называют версию;
установка или переустановка печатают команды `claude`, которые были бы выполнены:

```text
$ rtok agents update claude --cli --no-restart --dry-run
CLI: Claude Code — dry run, nothing written
plugin rtok@rtok 0.10.0 up to date (github)
$ rtok agents update claude --cli --no-restart --dry-run --force
CLI: Claude Code — dry run, nothing written
offer plugins/claude → claude plugin uninstall rtok@rtok && claude plugin install rtok@rtok ketch install pyrlyn/rtok
```

Более старая доступная версия (квитанция на `0.11.0`, rtok на `0.10.0`) остаётся на месте:

```text
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
plugin rtok@rtok: available 0.10.0 is older than installed 0.11.0
```

`--force` переустанавливает независимо от версий, включая понижение, и перезаписывает квитанцию.
`--source github|local|marketplace` сравнивает с этим источником вместо записанного;
другой источник означает переустановку, и квитанция переключается на него. После этого новый локальный
коммит или правка отслеживаемого файла — это обновление на месте (второй запуск ниже был после
правки):

```text
$ rtok agents update claude --cli --no-restart --source local
CLI: Claude Code
+ plugin plugins/claude → rtok@rtok
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
~ plugin rtok@rtok updated to 0.10.0+g84a4b607.dirty
```

**Когда переустановка не удалась.** Сбой `claude` до того, как что-либо было удалено, сохраняет старый плагин
и квитанцию, печатает строку `offer … (claude failed: …)` и завершается с кодом 0. Если удаление
прошло успешно, а установка — нет, у хоста нет плагина: rtok сообщает об этом, удаляет строку квитанции
и завершается с ненулевым кодом. Следующий `agents update` не видит ничего установленного и выполняет установку:

```text
$ rtok agents update claude --cli --no-restart --force
CLI: Claude Code
plugin rtok@rtok removed, reinstall failed: install failed
  ✗ plugin  not installed
  warning: plugin did not read back as installed
Error: a plugin reinstall failed
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
+ plugin plugins/claude → rtok@rtok
```

## Список устаревших плагинов

`rtok agents outdated` перечисляет хосты, чей плагин rtok старше запущенного rtok, и только их.
`rtok agents update --check` печатает ровно то же самое — для тех, кто ищет под `update`. Команда
читает только локальные файлы: никакой сети, никакого вызова CLI хоста и никакого обновления
marketplace, поэтому она быстрая и работает офлайн. Целевая версия — всегда собственная версия
запущенного бинарника (`rtok --version`).

```console
$ rtok agents outdated
agent  installed available source
claude 0.0.1     0.14.0    github

run: rtok agents update claude
```

**Что перечисляется.** Проверяется каждый хост, который поддерживает rtok (тот же реестр, что у
`agents list`), а не только те, что есть в квитанции, поэтому плагин, установленный вручную или
более старым rtok, тоже находится. Вариант хоста попадает в строку, когда там установлен плагин и его версия
ниже запущенного rtok по старшинству SemVer. Метаданные сборки игнорируются: локальная `0.14.0+g12c7e91`
при rtok `0.14.0` считается актуальной. Установленная версия ищется в том же порядке, что и в
`agents update` ([решение](#решение)): файл `.rtok-plugin-version` в установленной
копии, затем квитанция, затем собственная запись хоста (`installed_plugins.json` у Claude). Столбец
`source` берётся из того же поиска (`github`, `local`, `marketplace`).

**Что скрыто.** Хосты без плагина, с той же версией и с более новой версией
не печатаются. Хост со старой версией, который установлен, но версию которого нигде не
записано (нет файла версии, нет строки квитанции, нет пригодной записи хоста), считается `0.0.0` и показывается как
`legacy`; хост, чья запись называет версию, как `0.0.1` выше, показывает эту версию.

```console
$ rtok agents outdated
agent  installed available source
claude legacy    0.14.0    github

run: rtok agents update claude
```

**Делать нечего.** Два сообщения, в зависимости от того, что-то установлено или нет:

```console
$ rtok agents outdated
all rtok plugins are up to date (1 installed, rtok 0.14.0)
$ rtok agents outdated gemini
no rtok plugins installed
```

**Выбор хостов.** Как у `update`: необязательный список хостов через запятую
(`rtok agents outdated claude,cursor`) и `--cli` / `--desktop` для одного варианта.

**`--json`** печатает один объект и никакого сообщения для человека, в том числе когда обновлять нечего
(тогда `outdated` пуст, а `installed` считает проверенные плагины):

```console
$ rtok agents outdated --json
{"rtok":"0.14.0","outdated":[{"agent":"claude","variant":"cli","installed":"0.0.1","available":"0.14.0","source":"github","legacy":false}],"installed":1}
```

| Поле | Значение |
| --- | --- |
| `rtok` | Версия запущенного rtok, с которой сравнивается каждая строка. |
| `outdated[].agent`, `.variant` | Id хоста и вариант (`cli` или `desktop`). |
| `outdated[].installed` | Установленная версия или `legacy`, когда её нигде не записано. |
| `outdated[].available` | Версия запущенного rtok. |
| `outdated[].source` | `github`, `local` или `marketplace`. |
| `outdated[].legacy` | `true` для строки `legacy`. |
| `installed` | Сколько установок плагинов проверено, устаревших или нет. |

**`--exit-code`** завершается с кодом 10, когда устарел хотя бы один хост, и с 0 в остальных случаях. Без него
код выхода в обоих случаях 0, поэтому скрипт, который только читает вывод, продолжает работать:

```console
$ rtok agents outdated --json --exit-code; echo "exit=$?"
{"rtok":"0.14.0","outdated":[{"agent":"claude","variant":"cli","installed":"0.0.1","available":"0.14.0","source":"github","legacy":false}],"installed":1}
exit=10
```

**Почему это работает офлайн.** Доступная версия — собственная версия запущенного бинарника: тег или
запись каталога, соответствующие этой сборке, несут тот же `.rtok-plugin-version`
(это обеспечивает `tools/plugin-versions.sh --check`), поэтому спрашивать сервер не у кого. Отличаться от бинарника
может только локальная копия репозитория, и её читает `update`; `outdated` — нет.
Команда никогда ничего не меняет: чтобы действовать по списку, выполните строку `run:`, которую она печатает.

## Установленная версия в `agents list`

Строка `plugin` в `rtok agents list` и `rtok agents info <host>` называет установленную версию и
её источник; значение читается тем же поиском, что и в `agents outdated`. Если версия отличается
от запущенного rtok, строка говорит об этом, а для более старой называет команду, которая это
исправит; установка, возраст которой не определяют никакие файлы, показывается как `legacy`.
`rtok web` показывает тот же текст на странице Hosts, более старый плагин помечен `outdated`.

```console
$ rtok agents list
CLI: Claude Code
  ✓ plugin  installed 0.14.0 (marketplace), rtok is 0.15.1 — rtok agents update claude
$ rtok agents list
CLI: Claude Code
  ✓ plugin  installed (legacy, no version)
```

С `--json` каждый вариант хоста несёт `plugin_version` и `plugin_source`; оба отсутствуют, если
плагин не установлен или версия нигде не записана.

## Выпуск релиза

`tools/plugin-versions.sh` — единственное место, которое пишет и проверяет каждый файл версии и
манифест:

```text
$ tools/plugin-versions.sh --set 0.10.1
$ cat plugins/claude/.rtok-plugin-version
{"schema":1,"plugin":"claude","version":"0.10.1"}
$ tools/plugin-versions.sh --check 0.10.1
$ echo $?
0
```

`--check <version>` печатает каждый отличающийся файл и завершается с кодом 1; `--files` печатает каждый файл, который
затрагивает скрипт.

- `tools/release.sh` (запускается `just release` и `bump.yml`) вызывает `--set` в том же
  коммите `release: v<version>`, который повышает `Cargo.toml` и `Cargo.lock`, и индексирует файлы,
  которые перечисляет `--files`.
- `ci.yml` запускает `--check` по версии из `Cargo.toml` на каждом пул-реквесте, кроме черновиков, и
  на каждом push в `main`.
- `release.yml` запускает `--check` по тегу без `v` перед сборкой; несовпадение проваливает
  релиз.
- `tests/plugin_versions.rs` проверяет, что каждый файл из списка `--files` равен `CARGO_PKG_VERSION`, поэтому
  `just check` ловит расхождение локально.

Больше ничто не повышает версию: release-plz больше не запускается в CI, а пул-реквест
рабочего процесса Bump несёт коммит `release.sh`, поэтому проверка в `ci.yml` выполняется на нём до слияния,
а проверка в `release.yml` всё равно остановит тег, который проскочил. Сам процесс выпуска описан в
[Выпуске релизов rtok](../release.md).

## Устранение неполадок

**Плагин остаётся старым после `agents update`.** Сначала посмотрите на решение:

```text
$ rtok agents update claude --cli --no-restart --dry-run
CLI: Claude Code — dry run, nothing written
~ plugin rtok@rtok → 0.10.0 (claude plugin marketplace update rtok && claude plugin update rtok@rtok)
```

Если пробный запуск хочет обновить, а настоящий запуск говорит `already current`, значит, `claude` нет в
`PATH`: обновление пропускается, а квитанция остаётся как была. Добавьте `claude` в `PATH` и запустите
снова. Если пробный запуск говорит `up to date`, но копия, которую запускает Claude, старая, значит, квитанция неверна
(rtok доверяет ей больше, чем установленной копии): `rtok agents update claude --force`.

**Устаревшая установка.** У установки, сделанной до rtok 0.10.0, нет квитанции, а запись хоста —
`0.0.1` (или вовсе не SemVer). Первый `rtok agents update claude` обновляет её один раз и записывает
квитанцию; больше ничего не нужно. Если и это обновление, и запасная переустановка не удались после
удаления, см. «Когда переустановка не удалась» выше: перезапустите `rtok agents update claude`.

**Несовпадение версий в CI.** Шаг `plugin-versions.sh --check` или `tests/plugin_versions.rs`
падает, когда манифест или файл версии отредактировали вручную:

```text
$ tools/plugin-versions.sh --check 0.10.1
plugins/gemini/gemini-extension.json: 0.0.2 (want 0.10.1)
$ echo $?
1
```

Перепишите каждый файл по `Cargo.toml` и закоммитьте результат:

```bash
tools/plugin-versions.sh --set "$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
```

**`--source local` падает с `git describe`.** Локальный источник читает метаданные сборки из
git, поэтому ему нужно, чтобы дерево плагина находилось внутри checkout git; `plugins/` установленного rtok —
не такой checkout. Запустите бинарник rtok, собранный из checkout, или используйте `--source github`.
