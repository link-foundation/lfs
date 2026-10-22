# GitHub ideas about a file system and a database with embedded files (raw collection)

Collected on 2026-10-22 for [link-foundation/lfs#1](https://github.com/link-foundation/lfs/issues/1). This file only records ideas that were written down on GitHub. It does not evaluate them. Issue lfs#1 and PR lfs#2 themselves are excluded, as are CI and template boilerplate.

Relevant items recorded: **154** primary entries (issues, PRs, comments, wiki pages, one gist and two docs files) in 8 themes, plus **30** implementation PRs listed as secondary sources. Machine-readable copies are in `github-search-issues.json`, `github-search-discussions.json`, `github-search-code.json` and `github-repo-dump-hits.json` next to this file.

## Method

1. **Issue and PR search** (`gh api search/issues`, 100 per page, up to 10 pages, 2.2 s between calls) for each keyword below in `org:link-foundation`, `org:linksplatform`, `user:konard`, `org:deep-foundation`, `org:link-assistant`. Then extra global searches with `author:konard`.
2. **Discussion search** (GraphQL `search(type: DISCUSSION)`) for 23 keywords in the same five owners.
3. **Code search** (`gh api search/code`, `extension:md`) for README/docs mentions.
4. **Full dumps** of all issues and PRs (titles and bodies) of 124 repositories whose names or descriptions relate to links storage, memory, data, Deep or file systems, grepped locally with a regex covering file system, FUSE, Dokan, WinFsp, inode, ext4, blob, binary data, raw data, bitstring, journal, transaction, fragmentation, variable size, split/united/unified, memory-mapped, disk map, block size and the Russian forms (файловая система, хранилище, журнал, транзакция, фрагментация, ...). Hits are in `github-repo-dump-hits.json`.
5. **Wikis cloned**: `konard/LinksPlatform.wiki` (commit 98628fe, 2024-02-02) and `deep-foundation/dev.wiki`.
6. Every candidate below had its body read; comments were read for the items where they carry the idea (LinksPlatform #12, #216, #675; Data.Doublets #179, #185, #508). Titles alone were never used to infer content; items whose body is empty are marked "title only".

### Limits and gaps

- Search API: 30 requests per minute for issues, 10 per minute for code; at most 1000 results per query. Queries that hit the cap are shown as `total (got 1000)`; for them the full repo dumps (method 4) are the authoritative source.
- One secondary rate limit was hit (`"links storage" author:konard`, page 2); the script waited and retried, and the query completed (207 of 207). The `author:konard` searches are global, so they also return AI-solver PRs that konard's account opened in third-party repositories.
- Code search returns at most 100 results per query here (`got 100`), and only indexes default branches.
- Discussions: only `deep-foundation` has any matching discussions; none of the 9 returned discussions is about a file system or embedded files (titles listed under "Reviewed and excluded").
- Gists: `users/konard/gists` returns HTTP 502 at `per_page=100` after a few pages, so it was paged with `per_page=30` (pages 1-120 and 990-1032 of 1032; about 31 000 gists in total, almost all of them 2025-2026 AI solver logs). Gists from 2025-10 back to 2013-04 were all listed; 257 non-log gists before 2025-09 were filtered by keyword and 10 candidates were read. One relevant gist was found (item in MEM). Pages 121-989 (2025-10 to 2026-07) were not listed, but were spot-checked at pages 121, 125, 150, 200, 300, 500, 800 and 1000, and they are AI solver logs.
- Repositories with issues disabled (no issue data): linksplatform/conan-center-index, cs2py, java2cpp, ccc; konard/space_db, xlab2016-space_db_public, uselessgoddess-memory-layout, uselessgoddess-dunes, uselessgoddess-sandbox. konard/macos-ram-volume and konard/disk-analyzer have no issues or PRs.
- https://github.com/deep-foundation/dev/issues/125 (referenced by deep-packages#31 as the earlier variant of disk sync) does not resolve: it was deleted or transferred.
- link-foundation/deep, deeplinks, deep-packages, deepcase, documentation and hasura are mostly 2024-09 copies made by `konard/gh-org-migrator` of deep-foundation repos. The original deep-foundation URL is given as the source where it exists.

## Searches run

### Issues and pull requests (`search/issues`)

| keyword | org:link-foundation | org:linksplatform | user:konard | org:deep-foundation | org:link-assistant |
|---|---|---|---|---|---|
| `"file system"` | 45 | 23 | 68 | 4 | 127 |
| `filesystem` | 57 | 3 | 18 | 1 | 89 |
| `FUSE` | 15 | 7 | 2 | 0 | 20 |
| `Dokan` | 1 | 0 | 0 | 0 | 0 |
| `WinFsp` | 0 | 0 | 0 | 0 | 0 |
| `files` | 1133 (got 1000) | 490 | 843 | 114 | 2183 (got 1000) |
| `file` | 2362 (got 1000) | 891 | 1332 (got 1000) | 308 | 4105 (got 1000) |
| `"embedded files"` | 6 | 7 | 0 | 0 | 9 |
| `"file storage"` | 43 | 25 | 57 | 9 | 27 |
| `blob` | 392 | 931 | 177 | 230 | 255 |
| `"binary files"` | 23 | 2 | 18 | 2 | 23 |
| `"raw data"` | 27 | 24 | 35 | 1 | 245 |
| `storage` | 207 | 121 | 286 | 48 | 207 |
| `database` | 107 | 63 | 184 | 40 | 89 |
| `journal` | 16 | 7 | 88 | 0 | 36 |
| `transaction` | 24 | 32 | 57 | 12 | 42 |
| `bitstring` | 2 | 31 | 7 | 0 | 0 |
| `"bit string"` | 3 | 2 | 5 | 0 | 0 |
| `"split links"` | 11 | 19 | 8 | 0 | 11 |
| `"unified links"` | 7 | 2 | 6 | 0 | 9 |
| `"united links"` | 0 | 9 | 0 | 0 | 0 |
| `split` | 203 | 109 | 98 | 14 | 360 |
| `united` | 1 | 42 | 2 | 0 | 9 |
| `"variable size"` | 5 | 2 | 10 | 1 | 3 |
| `fragmentation` | 2 | 4 | 4 | 1 | 1 |
| `defragmentation` | 0 | 0 | 0 | 0 | 0 |
| `disk` | 235 | 47 | 68 | 7 | 436 |
| `block` | 829 | 1975 (got 1000) | 476 | 88 | 1062 (got 1000) |
| `inode` | 9 | 1 | 6 | 0 | 9 |
| `ext4` | 2 | 0 | 0 | 0 | 0 |
| `NTFS` | 0 | 0 | 0 | 0 | 0 |
| `mount` | 59 | 2 | 16 | 5 | 143 |
| `"файловая система"` | 0 | 0 | 0 | 0 | 0 |
| `"файловой системы"` | 1 | 0 | 0 | 1 | 0 |
| `файл` | 2 | 2 | 3 | 5 | 20 |
| `файлы` | 3 | 1 | 5 | 1 | 10 |
| `файлов` | 2 | 1 | 3 | 2 | 15 |
| `файловая` | 1 | 0 | 0 | 0 | 0 |
| `хранилище` | 2 | 1 | 1 | 2 | 3 |
| `хранения` | 1 | 0 | 8 | 1 | 2 |
| `"база данных"` | 0 | 0 | 5 | 0 | 0 |
| `"базы данных"` | 0 | 0 | 6 | 0 | 1 |
| `журнал` | 0 | 0 | 0 | 0 | 2 |
| `транзакции` | 2 | 1 | 2 | 2 | 0 |
| `индекс` | 0 | 1 | 1 | 0 | 0 |
| `фрагментация` | 0 | 0 | 0 | 0 | 0 |
| `дефрагментация` | 0 | 0 | 1 | 0 | 0 |
| `диск` | 0 | 0 | 5 | 0 | 1 |
| `диска` | 0 | 0 | 0 | 0 | 1 |
| `блоки` | 0 | 0 | 2 | 0 | 0 |

### Global `author:konard` searches

| keyword | author:konard |
|---|---|
| `filesystem` | 259 |
| `"file system"` | 469 |
| `FUSE` | 64 |
| `Dokan` | 1 |
| `"файловая система"` | 7 |
| `"файловой системы"` | 14 (got 13) |
| `doublets` | 643 |
| `"links storage"` | 207 |
| `"links database"` | 121 |
| `"links file system"` | 20 |

### Discussions (GraphQL)

| keyword | org:link-foundation | org:linksplatform | user:konard | org:deep-foundation | org:link-assistant |
|---|---|---|---|---|---|
| `"file system"` | 0 | 0 | 0 | 0 | 0 |
| `filesystem` | 0 | 0 | 0 | 0 | 0 |
| `FUSE` | 0 | 0 | 0 | 0 | 0 |
| `Dokan` | 0 | 0 | 0 | 0 | 0 |
| `files` | 0 | 0 | 0 | 0 | 0 |
| `file` | 0 | 0 | 0 | 4 | 0 |
| `storage` | 0 | 0 | 0 | 1 | 0 |
| `database` | 0 | 0 | 0 | 1 | 0 |
| `journal` | 0 | 0 | 0 | 0 | 0 |
| `transaction` | 0 | 0 | 0 | 0 | 0 |
| `bitstring` | 0 | 0 | 0 | 0 | 0 |
| `disk` | 0 | 0 | 0 | 0 | 0 |
| `block` | 0 | 0 | 0 | 0 | 0 |
| `blob` | 0 | 0 | 0 | 2 | 0 |
| `fragmentation` | 0 | 0 | 0 | 0 | 0 |
| `"файловая система"` | 0 | 0 | 0 | 0 | 0 |
| `файл` | 0 | 0 | 0 | 0 | 0 |
| `файлы` | 0 | 0 | 0 | 0 | 0 |
| `хранилище` | 0 | 0 | 0 | 1 | 0 |
| `"база данных"` | 0 | 0 | 0 | 0 | 0 |
| `журнал` | 0 | 0 | 0 | 0 | 0 |
| `индекс` | 0 | 0 | 0 | 0 | 0 |
| `диск` | 0 | 0 | 0 | 0 | 0 |

### Code (`search/code`, `extension:md`)

| keyword | org:link-foundation | org:linksplatform | user:konard | org:deep-foundation | org:link-assistant |
|---|---|---|---|---|---|
| `"file system"` | 6 | 1 | 32 | 2 | 22 |
| `filesystem` | 46 | 0 | 62 | 0 | 248 (got 100) |
| `FUSE` | 55 | 8 | 224 (got 100) | 0 | 800 (got 100) |
| `Dokan` | 0 | 0 | 0 | 0 | 0 |
| `"embedded files"` | 0 | 0 | 0 | 0 | 0 |
| `"файловая система"` | 0 | 0 | 37 | 0 | 1 |
| `"файловой системы"` | 0 | 0 | 17 | 0 | 2 |
| `"файловую систему"` | 0 | 0 | 2 | 0 | 0 |

### Repositories dumped in full

`linksplatform/Comparisons.SQLiteVSDoublets: issues=14 prs=110`, `linksplatform/doublets-rs: issues=35 prs=37`, `linksplatform/Comparisons.DeepGqlStandard: issues=2 prs=3`, `linksplatform/Comparisons.SpacetimeDBVSDoublets: issues=5 prs=12`, `linksplatform/Comparisons.RedisVSDoublets: issues=2 prs=3`, `linksplatform/Comparisons.PostgreSQLVSDoublets: issues=9 prs=20`, `linksplatform/Interfaces: issues=56 prs=90`, `linksplatform/Comparisons.Neo4jVSDoublets: issues=9 prs=9`, `linksplatform/Delegates: issues=19 prs=16`, `linksplatform/doublets-web: issues=3 prs=30`, `linksplatform/Solver: issues=4 prs=7`, `linksplatform/Data.Doublets: issues=210 prs=304`, `linksplatform/Numbers: issues=38 prs=102`, `linksplatform/data-rs: issues=9 prs=9`, `linksplatform/trees-rs: issues=15 prs=16`, `linksplatform/mem-rs: issues=9 prs=27`, `linksplatform/Scripts: issues=12 prs=25`, `linksplatform/RegularExpressions.Transformer.CSharpToCpp: issues=23 prs=71`, `linksplatform/aqlgen: issues=2 prs=0`, `linksplatform/Crawler: issues=16 prs=111`, `linksplatform/Disposables: issues=29 prs=69`, `linksplatform/Collections: issues=57 prs=125`, `linksplatform/Random: issues=12 prs=48`, `linksplatform/Exceptions: issues=28 prs=51`, `linksplatform/Unsafe: issues=17 prs=79`, `linksplatform/Threading: issues=14 prs=59`, `linksplatform/Protocols: issues=5 prs=56`, `linksplatform/IO: issues=20 prs=78`, `linksplatform/Memory: issues=24 prs=65`, `linksplatform/Ranges: issues=34 prs=57`, `linksplatform/Timestamps: issues=15 prs=51`, `linksplatform/Converters: issues=17 prs=75`, `linksplatform/Collections.Methods: issues=28 prs=84`, `linksplatform/Data: issues=49 prs=125`, `linksplatform/Singletons: issues=4 prs=48`, `linksplatform/Data.Doublets.Sequences: issues=10 prs=31`, `linksplatform/Reflection: issues=26 prs=65`, `linksplatform/RegularExpressions.Transformer: issues=12 prs=46`, `linksplatform/Data.Triplets.Kernel: issues=16 prs=18`, `linksplatform/RegularExpressions.Transformer.CppToJava: issues=3 prs=3`, `linksplatform/Bot: issues=146 prs=213`, `linksplatform/Setters: issues=7 prs=49`, `linksplatform/Documentation: issues=7 prs=22`, `linksplatform/Data.Triplets: issues=4 prs=76`, `linksplatform/Incrementers: issues=10 prs=40`, `linksplatform/InfiniteDepthTreeWebUIPrototype: issues=16 prs=12`, `linksplatform/Scopes: issues=2 prs=44`, `linksplatform/RawDoubletsViewer: issues=2 prs=2`, `linksplatform/CSharpToCppTranslatorTestSolution: issues=1 prs=1`, `linksplatform/react-deep-tree: issues=8 prs=18`, `linksplatform/RegularExpressions.Transformer.HasuraSQLSimplifier: issues=7 prs=29`, `linksplatform/Comparers: issues=3 prs=41`, `linksplatform/Examples.Doublets.CRUD: issues=9 prs=34`, `linksplatform/Data.Doublets.Xml: issues=6 prs=29`, `linksplatform/linksplatform.github.io: issues=2 prs=2`, `linksplatform/Hashing: issues=9 prs=16`, `linksplatform/Data.Doublets.Json: issues=11 prs=43`, `linksplatform/Data.Doublets.Gql: issues=19 prs=43`, `linksplatform/Data.Doublets.Sql: issues=1 prs=2`, `linksplatform/CSharpToCppTranslator: issues=1 prs=98`, `linksplatform/Settings: issues=8 prs=6`, `linksplatform/Data.Sequences: issues=2 prs=11`, `linksplatform/Workflows: issues=5 prs=3`, `linksplatform/DoubletsVsClickHouseBenchmarks: issues=2 prs=1`, `linksplatform/Data.Doublets.Lino: issues=1 prs=9`, `linksplatform/.github: issues=1 prs=1`, `linksplatform/Data.Doublets.Gql.Client: issues=0 prs=1`, `linksplatform/RegularExpressions.Transformer.CSharpToPython: issues=0 prs=1`, `linksplatform/Diagnostics: issues=0 prs=32`, `linksplatform/Hardware.Cpu: issues=0 prs=35`, `linksplatform/Files: issues=0 prs=1`, `linksplatform/Equality: issues=0 prs=2`, `linksplatform/CodeNetJavaToPythonTranslationExperiment: issues=0 prs=1`, `linksplatform/conan-center-index: issues=0 prs=34`, `linksplatform/RegularExpressions.Transformer.PythonToNim: issues=0 prs=0`, `linksplatform/IdClient: issues=0 prs=0`, `linksplatform/IdDistributorServer: issues=0 prs=0`, `linksplatform/resourse-cooperation: issues=0 prs=0`, `linksplatform/doublets-dev: issues=0 prs=0`, `linksplatform/cs2py: issues=0 prs=0`, `linksplatform/java2cpp: issues=0 prs=0`, `linksplatform/ccc: issues=0 prs=2`, `linksplatform/platform-rs: issues=0 prs=0`, `linksplatform/Reflection.Sigil: issues=6 prs=5`, `linksplatform/Sigil: issues=0 prs=0`, `linksplatform/BareJS: issues=2 prs=0`, `linksplatform/BareNET: issues=1 prs=0`, `linksplatform/Helpers: issues=10 prs=3`, `konard/LinksPlatform: issues=477 prs=579`, `konard/space_db: issues=0 prs=0`, `konard/xlab2016-space_db_public: issues=0 prs=0`, `konard/tasks-registry: issues=0 prs=0`, `konard/uselessgoddess-memory-layout: issues=0 prs=0`, `konard/uselessgoddess-dunes: issues=0 prs=0`, `konard/uselessgoddess-sandbox: issues=0 prs=0`, `konard/DoubletsInnerStructureExplanation: issues=0 prs=0`, `konard/links-visuals: issues=13 prs=19`, `konard/macos-ram-volume: issues=0 prs=0`, `konard/disk-analyzer: issues=0 prs=0`, `link-foundation/lfs: issues=1 prs=1`, `link-foundation/link-cli: issues=65 prs=63`, `link-foundation/links-notation: issues=91 prs=261`, `link-foundation/links-queue: issues=27 prs=26`, `link-foundation/linksql: issues=1 prs=2`, `link-foundation/links-client: issues=12 prs=12`, `link-foundation/disk-space-saviour: issues=26 prs=19`, `link-foundation/links-logs-vault: issues=1 prs=2`, `link-foundation/meta-theory: issues=31 prs=30`, `link-foundation/box: issues=66 prs=68`, `link-foundation/ideav-local-associative-js: issues=2 prs=2`, `link-foundation/deep: issues=0 prs=0`, `link-foundation/deeplinks: issues=210 prs=0`, `link-foundation/deep-theory: issues=4 prs=0`, `link-foundation/lino-objects-codec: issues=33 prs=41`, `link-foundation/lino-cache: issues=2 prs=2`, `link-foundation/start: issues=129 prs=116`, `link-foundation/relative-meta-logic: issues=93 prs=94`, `deep-foundation/deeplinks: issues=283 prs=50`, `deep-foundation/deep-theory: issues=4 prs=1`, `deep-foundation/Discussions: issues=1 prs=0`, `deep-foundation/dev: issues=186 prs=30`, `deep-foundation/deepcase: issues=70 prs=11`, `link-assistant/hive-mind: issues=1783 prs=1911`, `link-assistant/formal-ai: issues=776 prs=614`

## Ideas by theme

### File system on links (DB is the FS, LFS, tag FS, metadata in links) (18)

1. **[konard/LinksPlatform#12](https://github.com/konard/LinksPlatform/issues/12)** - issue, created 2015-09-13, open, by konard: "Allow store raw/binary data inside links database"
   - Quote: "By allowing storage of any raw files we can cover any possible need in the future for storing, and allow database to allocate the whole drives, so DB itself becomes FileSystem."
   - Idea: Store raw numbers and raw files inside the links DB as continuous blocks of links (block size a multiple of the link size); with whole-drive allocation the DB itself becomes a file system.
2. **[konard/LinksPlatform#12](https://github.com/konard/LinksPlatform/issues/12#issuecomment-168984941)** - comment on issue (thread created 2015-09-13; comment date in the note), open, by konard: "Allow store raw/binary data inside links database"
   - Quote: "If length of the file is unknown it is possible to write it at the end of database until we done. So the gaps between files are always filled with links, not smaller files."
   - Idea: Files of unknown length are appended at the end of the DB; gaps between files are always filled with links, never with smaller files.
   - Note: comment by konard, 2016-01-05, on #12
3. **[konard/LinksPlatform#12](https://github.com/konard/LinksPlatform/issues/12#issuecomment-710975028)** - comment on issue (thread created 2015-09-13; comment date in the note), open, by konard: "Allow store raw/binary data inside links database"
   - Quote: "To determine whether a link or raw data block exists it might be helpful to use bit strings for mapping the used space. One bit string for links (...) and one bit string for raw data block (...). Another solution is to use a tree for raw data blocks indices ranges, but if used alone can be slow."
   - Idea: Two bitstrings map used space (one for links, one for raw data blocks); a tree of raw-block ranges is the alternative.
   - Note: comment by konard, 2020-10-17, on #12; other comments on #12: 2015-12-31 file marker '(x: x (file 0d5839))' that can be re-deduced if lost, 2016-02-05 last link index as start/end marker so the smallest file fits one link, 2016-02-12 separate trees or memory managers for raw data, 2016-07-21 'Best with #213 (makes file system out of LinksPlatform)'
4. **[konard/LinksPlatform#216](https://github.com/konard/LinksPlatform/issues/216#issuecomment-619353772)** - comment on issue (thread created 2016-08-04; comment date in the note), open, by konard: "Links File System"
   - Quote: "Regular file systems are not so good at treating small files. So Doublets can be used to store files along side with data and text represented by links with deduplication. In this way it is actually combination of database and a file system in single file or the whole drive allocation."
   - Idea: Links File System: a database and a file system in one file or whole drive, good for small files, with deduplication; depends on #12 and #213. Body cites pifs (pi file system) and a Library-of-Babel style addressing alternative.
   - Note: quote is konard's comment of 2020-04-25; lubyagin replied 2020-05-01 (RU): 'Можно разнести мелкие файлы на отдельный storage, а блоки крупных файлов - на большой.' = 'Small files can be put on a separate storage, and blocks of large files on a big one.'
5. **[konard/LinksPlatform#213](https://github.com/konard/LinksPlatform/issues/213)** - issue, created 2016-07-21, open, by konard: "The "whole drive" fixed sized links storage allocation (no file system)"
   - Quote: "This can be a way "not to use" memory mapped files an go to BareMetal OS (https://github.com/ReturnInfinity/BareMetal-OS) as soon as possible."
   - Idea: Whole-drive, fixed-size links storage allocation with no host file system underneath.
6. **[linksplatform/Data.Doublets#508](https://github.com/linksplatform/Data.Doublets/issues/508)** - issue, created 2025-09-25, open, by konard: "Allow treat links references as binary data"
   - Quote: "That will also help allow to store variable length references sequences inside the doublets store, which in turn allow us to use links store as a file system, archive or file storage database."
   - Idea: Allow references to be treated as binary data (broken references allowed, reference checks scoped), so the links store can serve as a file system, archive or file storage DB.
7. **[linksplatform/Data.Doublets#512](https://github.com/linksplatform/Data.Doublets/issues/512)** - issue, created 2026-05-23, open, by konard: "UnitedRangedMemoryLinks that will support ranges for binary data"
   - Quote: "And all space between raw binary data block will be filled with links, if we have unused or deleted range of links inside the used data, and it is enough for requested binary range, we should use it from there, so we prefer filling the empty / unused space, to pack up everything nicely. And yet, we should never allow fragmentation, meaning if the size of requested range is greater than any free range, we should just append it to the end of the data store."
   - Idea: UnitedRangedMemoryLinks: free list of ranges instead of single links; allocate ranges of links and raw binary ranges (marker constant + length in TLinkAddress units); gaps filled with links; no fragmentation, oversized requests appended at the end.
8. **[konard/LinksPlatform#176](https://github.com/konard/LinksPlatform/issues/176)** - issue, created 2016-04-21, open, by konard: "Tag file system"
   - Quote: "One file, multiple paths an tags. All files in one collection really. Paths can be implemented as tag inheritance."
   - Idea: Tag file system: one collection of files, multiple paths and tags per file, paths as tag inheritance.
9. **[deep-foundation/documentation#27](https://github.com/deep-foundation/documentation/issues/27)** - issue, created 2024-01-04, open, by konard: "Make a list of key advantages"
   - Quote: "поддержка функций файловой системы (реализованная Contain связями) с возможностью размещать одну и ту же связь в двух предках ("папках"), что доступно не в каждой ОС **(доставлено)**"
   - Translation: "support for file-system functions (implemented with Contain links) with the ability to place the same link in two ancestors ("folders"), which is not available in every OS (delivered)"
   - Idea: Deep already implements file-system functions with Contain links, and one link can live in two parent folders (multi-parent directories).
   - Note: mirrored as https://github.com/link-foundation/documentation/issues/2 (2024-09-03)
10. **[konard/LinksPlatform#641](https://github.com/konard/LinksPlatform/issues/641)** - issue, created 2022-03-03, open, by konard: "Recursive path can be written as recursive links network structure"
   - Quote: "It may be possible to write down a recursive path (as in materialized path), as links network, not tree. Path as regular sequence vs recursive sequence."
   - Idea: Represent paths (materialized paths) as a recursive links structure instead of a tree.
11. **[konard/LinksPlatform#699](https://github.com/konard/LinksPlatform/issues/699)** - issue, created 2025-10-16, open, by konard: "Metadata Storage in Semantic Network Database"
   - Quote: "Store file metadata in a semantic network database instead of traditional file systems."
   - Idea: File metadata (and metadata of network FS such as CIFS/SMB/NFS) lives in a semantic-network (links) DB.
   - Note: moved from the wiki idea 11, see the wiki row below
12. **[konard/LinksPlatform](https://github.com/konard/LinksPlatform/wiki/Ideas,-%D0%B8%D0%B4%D0%B5%D0%B8)** - wiki, created wiki clone, n/a, by konard and contributors (pacify signs some items): "Ideas, идеи (wiki page)"
   - Quote: "Часть метаинформации полезно хранить не в файловой системе (директориях, именах/содержимом файлов) - а в базе данных семантической сети. Тоже самое - для сетевых файловых систем (CIFS, SMB, NFS)."
   - Translation: "It is useful to store part of the meta-information not in the file system (directories, file names/contents) but in a semantic network database. The same applies to network file systems (CIFS, SMB, NFS)."
   - Idea: Original wiki idea 11 behind #699: keep part of the meta-information in a semantic-network DB rather than in directories and file names.
   - Note: wiki page; also BareMetal OS idea: 'Ограничить интенсивность страничного обмена' = 'Limit the intensity of page swapping'
13. **[konard/LinksPlatform#484](https://github.com/konard/LinksPlatform/issues/484)** - issue, created 2019-10-23, open, by konard: "Better recovery system for operating systems"
   - Quote: "File system should be like git/wikipedia - only changes are recorded. Recovery should work to any point in time. Related to #216."
   - Idea: A file system that records only changes, with recovery to any point in time.
14. **[konard/LinksPlatform#539](https://github.com/konard/LinksPlatform/issues/539)** - issue, created 2020-04-07, open, by konard: "Text storage"
   - Quote: "All text files stored in one place in a single database."
   - Idea: Text storage: all text files in one DB, synchronised with files on disk if needed, per-file version control; later part of #216.
15. **[konard/LinksPlatform#540](https://github.com/konard/LinksPlatform/issues/540)** - issue, created 2020-04-07, open, by konard: "Code storage"
   - Quote: "Should be able to export/import as git repositories and all other version controls."
   - Idea: Code storage in the links DB with import/export to git and other VCS.
16. **[konard/LinksPlatform#138](https://github.com/konard/LinksPlatform/issues/138)** - issue, created 2016-01-26, open, by konard: "Data importers"
   - Quote: "- [ ] File System importer"
   - Idea: Data importers, including a file-system importer into links.
17. **[konard/LinksPlatform#609](https://github.com/konard/LinksPlatform/issues/609)** - issue, created 2021-08-18, open, by konard: "A better translator architecture prototype"
   - Quote: "Once the AST and the FileSystem snapshot are extracted into Doublets as an intermediate storage all actual translation should be performed inside Doublets storage."
   - Idea: A file-system snapshot (project files as AST) stored in Doublets as the working store.
18. **[konard/LinksPlatform#669](https://github.com/konard/LinksPlatform/issues/669)** - issue, created 2023-06-01, open, by konard: "Better than git: source control that is not based on files but classes/functions."
   - Quote: "Better than git: source control that is not based on files but classes/functions."
   - Idea: Version control over a code graph instead of files.
   - Note: quote is the title

### File contents as links, sequences and raw binary data (17)

19. **[konard/LinksPlatform#675](https://github.com/konard/LinksPlatform/issues/675)** - issue, created 2023-06-28, open, by konard: "Links and Binary data"
   - Quote: "It may be required only to store links and binary data between links. Here in binary data we can store all indexes optionally. Including incoming and outgoing links."
   - Idea: A store of links plus binary data between them, where indexes are optional and kept in that binary data.
20. **[linksplatform/Data.Doublets#207](https://github.com/linksplatform/Data.Doublets/issues/207)** - issue, created 2021-05-31, open, by konard: "byte[] to sequence and back converters"
   - Quote: "byte[] to sequence and back converters"
   - Idea: Convert byte arrays to link sequences and back.
   - Note: title only, body empty
21. **[linksplatform/Data.Doublets#201](https://github.com/linksplatform/Data.Doublets/issues/201)** - issue, created 2021-05-19, open, by konard: "Check if it is possible to use bit by bit addressing of the binary tree sequence without size limitations"
   - Quote: "This way it will be possible to address any sequence with any structure, but it may not contain all indexes/addresses."
   - Idea: Bit-by-bit addressing of binary-tree sequences (0 = source, 1 = target, 2 = source.source, ...).
22. **[konard/LinksPlatform#555](https://github.com/konard/LinksPlatform/issues/555)** - issue, created 2020-10-09, open, by konard: "Software logs storage"
   - Quote: "Each file can be stored as doublets storage without indices and can reference shared dictionary for it's seque"
   - Idea: Logs stored as deduplicated doublets: one index-free doublets store per file plus a shared dictionary per application.
   - Note: quote truncated mid-sentence in the source dump
23. **[konard/LinksPlatform#304](https://github.com/konard/LinksPlatform/issues/304)** - issue, created 2019-04-04, open, by konard: "An even better fight with dublication on IPFS, BitTorrent and so on. "The fragment Id service"."
   - Quote: "Links can be a foundation for founding most used sequences fragments, that should be stored ones and for all in one place without any further duplication."
   - Idea: Fragment-id service: store frequent fragments once, deduplicating across IPFS, BitTorrent and similar systems.
24. **[konard/LinksPlatform#182](https://github.com/konard/LinksPlatform/issues/182)** - issue, created 2016-04-25, open, by konard: "Use sequences compession for indexing the Pi, or any other sequence, that can be used to encode source sequence"
   - Quote: "Use sequences compession for indexing the Pi, or any other sequence, that can be used to encode source sequence"
   - Idea: Index a known sequence (pi, as in pifs) with sequence compression and encode content as positions in it.
   - Note: quote is the title; body links philipl/pifs#33
25. **[konard/LinksPlatform#95](https://github.com/konard/LinksPlatform/issues/95)** - issue, created 2015-12-06, open, by konard: "Compression optimization"
   - Quote: "- [ ] Global dictionary / count of pairs across multiple compressions"
   - Idea: Compression optimisations for sequences, including a global dictionary shared across compressions.
26. **[linksplatform/Data.Doublets#97](https://github.com/linksplatform/Data.Doublets/issues/97)** - issue, created 2019-10-02, open, by konard: "Add an option for automatic compactification (deduplication) of all sequences on Sequences' Dispose"
   - Quote: "Add an option for automatic compactification (deduplication) of all sequences on Sequences' Dispose"
   - Idea: Deduplicate all sequences automatically on close.
   - Note: title only
27. **[konard/LinksPlatform#567](https://github.com/konard/LinksPlatform/issues/567)** - issue, created 2020-12-11, open, by konard: "Store variable length fragments in the same file"
   - Quote: "Store variable length fragments in the same file"
   - Idea: Variable-length fragments in the same file as the links, via a Fragments table (LinkAddress, FragmentContentOffset, FragmentContentLength) and a FragmentContent area.
   - Note: quote is the title
28. **[konard/LinksPlatform#205](https://github.com/konard/LinksPlatform/issues/205)** - issue, created 2016-07-21, open, by konard: "Binary mapper"
   - Quote: "Make a software that can map any binary file, and and also compare differences between two binary files."
   - Idea: Binary mapper: map any binary file onto a schema and diff two binary files.
29. **[konard/LinksPlatform#549](https://github.com/konard/LinksPlatform/issues/549)** - issue, created 2020-07-16, open, by konard: "Strings storage service based on Sequences"
   - Quote: "Strings storage service based on Sequences"
   - Idea: Strings stored as sequences of links.
   - Note: title only
30. **[konard/LinksPlatform#561](https://github.com/konard/LinksPlatform/issues/561)** - issue, created 2020-10-12, open, by konard: "Store internationalization data inside links file (one file for all languages)"
   - Quote: "Store internationalization data inside links file (one file for all languages)"
   - Idea: All i18n data in one links file.
   - Note: title only
31. **[deep-foundation/deeplinks#257](https://github.com/deep-foundation/deeplinks/issues/257)** - issue, created 2023-10-20, open, by konard: "Files support for packager"
   - Quote: "At the moment, the best guess is to use `base64` as the value of `AsyncFile` links. Later, we can support paths relative to `deep.json` or `deep.xml` instead of `base64` in `value` filed of the link."
   - Idea: Deep packager files: binary file content in the link value (base64) or as a relative path to an assets folder.
   - Note: mirrored as https://github.com/link-foundation/deeplinks/issues/60
32. **[deep-foundation/deeplinks#296](https://github.com/deep-foundation/deeplinks/issues/296)** - issue, created 2024-01-24, open, by konard: "Support URI for string values and AsyncFiles in Packager"
   - Quote: "That will allow to store files near deep.json or even in internet for example in CDNs or even on ipfs and blockchain."
   - Idea: URIs as values for AsyncFile links, so file content can live outside the DB (CDN, IPFS).
33. **[deep-foundation/deepcase#25](https://github.com/deep-foundation/deepcase/issues/25)** - issue, created 2023-10-30, open, by konard: "Drag & drop of binary files makes AsyncFile, make a drag & drop of text files produce SyncTextFile links"
   - Quote: "Drag & drop of binary files makes AsyncFile, make a drag & drop of text files produce SyncTextFile links"
   - Idea: Binary files become AsyncFile links; text files become SyncTextFile links whose content is in the DB.
   - Note: title only; mirrored as https://github.com/link-foundation/deepcase/issues/38
34. **[deep-foundation/deeplinks#142](https://github.com/deep-foundation/deeplinks/issues/142)** - issue, created 2023-04-04, open, by FreePhoenix888: "Add Value link from AsyncFile to File"
   - Quote: "If type has value there is Value link which points from type to `String|Number|Object`. We should do the same for `File`"
   - Idea: Treat File as a value kind for links, like String, Number and Object.
35. **[link-assistant/meta-expression#63](https://github.com/link-assistant/meta-expression/issues/63)** - issue, created 2026-05-25, closed, by konard: "Foundation: durable doublets-backed storage with Unicode sequences"
   - Quote: "Represent Unicode strings as sequences of character links."
   - Idea: Durable doublets-backed storage where strings are stored as sequences of character links, with save/load and export/import.

### Indexes: bitstrings vs trees, optional indexes (18)

36. **[konard/LinksPlatform#5](https://github.com/konard/LinksPlatform/issues/5)** - issue, created 2015-09-10, open, by konard: "Test alternative forms of index (lists of connections between links)"
   - Quote: "1. Bit index (bit string), with optimization for empty spaces."
   - Idea: Test alternative index forms: bitstring, AVL/B-tree, skip lists, matrix, linked lists, hash tables, or a hybrid.
37. **[konard/LinksPlatform#566](https://github.com/konard/LinksPlatform/issues/566)** - issue, created 2020-12-11, open, by konard: "Additional indexing layer using BitStrings"
   - Quote: "BitStrings can use many times less space than tries."
   - Idea: An extra indexing layer built on BitStrings.
38. **[konard/LinksPlatform#663](https://github.com/konard/LinksPlatform/issues/663)** - issue, created 2023-03-10, open, by konard: "Bit strings variations"
   - Quote: "We can have a smaller bit string that represent a bitmap with each 1 bit corresponding to N bits in original bit string."
   - Idea: Hierarchical bitstrings: a summary bitmap where 1 bit covers N bits, with granularity tied to the cache line or page.
39. **[linksplatform/Data.Doublets#185](https://github.com/linksplatform/Data.Doublets/issues/185#issuecomment-821825822)** - comment on issue (thread created 2020-12-28; comment date in the note), open, by konard: "Use bitstring/bitarray indexes"
   - Quote: "Each bit string can contain not only direct links with this link, but all direct and indirect usages of the link (its subtree of the graph). (...) Such bit strings can be a replacement of materialized path index."
   - Idea: Bitstring indexes per fragment or pair, intersected for search; subtree bitstrings replace the materialized-path index; bitstrings per value range.
   - Note: quote from konard's comment of 2021-04-17; comment of 2024-03-29: 'We can also generalize bit string indexes for ranges of values.'
40. **[linksplatform/Data.Doublets#240](https://github.com/linksplatform/Data.Doublets/issues/240)** - issue, created 2021-09-11, open, by konard: "Implement simple BitString/BitArray index layer/decorator that can be attached to links storage"
   - Quote: "This index should store the same data as tree indexes, so we will be able to compare which one is faster in what situations."
   - Idea: A configurable BitString index decorator kept in sync on every CRUD operation, to compare with the trees.
41. **[linksplatform/Data.Doublets#396](https://github.com/linksplatform/Data.Doublets/issues/396)** - issue, created 2023-11-06, open, by konard: "Support bitstring indexing as alternative or additional to trees"
   - Quote: "Support bitstring indexing as alternative or additional to trees"
   - Idea: Bitstrings as an alternative or an addition to the tree indexes.
   - Note: title only; PR https://github.com/linksplatform/Data.Doublets/pull/400
42. **[linksplatform/Data.Doublets#398](https://github.com/linksplatform/Data.Doublets/issues/398)** - issue, created 2024-02-29, open, by konard: "Try to use bitstring for a map of deleted links to test if it is faster"
   - Quote: "Try to use bitstring for a map of deleted links to test if it is faster"
   - Idea: Bitstring map of free (deleted) links instead of a linked list.
   - Note: title only; PR https://github.com/linksplatform/Data.Doublets/pull/399 claims 52x speed-up
43. **[linksplatform/Data.Doublets#395](https://github.com/linksplatform/Data.Doublets/issues/395)** - issue, created 2023-11-06, open, by konard: "Make Trees indexing optional"
   - Quote: "Indexing using trees may be moved to decorators"
   - Idea: Tree indexes become optional decorators.
44. **[linksplatform/Data.Doublets#239](https://github.com/linksplatform/Data.Doublets/issues/239)** - issue, created 2021-08-30, open, by konard: "Basic index can be implemented as a decorator/layer and can be optional or switchable"
   - Quote: "Basic index can be implemented as a decorator/layer and can be optional or switchable"
   - Idea: Basic index as an optional, switchable layer.
   - Note: title only
45. **[konard/LinksPlatform#15](https://github.com/konard/LinksPlatform/issues/15)** - issue, created 2015-09-15, open, by konard: "Allow to switch Platform.Data.Kernel.dll between linked-list and tree"
   - Quote: "Add some mark to distinguish between Linked list and Tree. So the database will not be damaged by opening."
   - Idea: Switch the kernel between linked-list and tree indexes, with a marker in the file.
46. **[konard/LinksPlatform#522](https://github.com/konard/LinksPlatform/issues/522)** - issue, created 2020-01-24, open, by konard: "Reduce the size of index tree by adding service link for every N links each service link will contain it's own trees"
   - Quote: "Reduce the size of index tree by adding service link for every N links each service link will contain it's own trees"
   - Idea: Split indexes into per-N-links subtrees.
   - Note: title only
47. **[konard/LinksPlatform#526](https://github.com/konard/LinksPlatform/issues/526)** - issue, created 2020-02-06, open, by konard: "Use two matrices as an indices for links"
   - Quote: "The first matrix is between link's address and the link's source. The second matrix is between link's address and the link's target."
   - Idea: Two matrices as indexes.
48. **[konard/LinksPlatform#313](https://github.com/konard/LinksPlatform/issues/313)** - issue, created 2019-04-14, open, by konard: "Bit by bit sequence indexing"
   - Quote: "This way even tiniest patterns will be extracted from any sequence."
   - Idea: Bit-by-bit sequence indexing.
49. **[konard/LinksPlatform#327](https://github.com/konard/LinksPlatform/issues/327)** - issue, created 2019-04-30, open, by konard: "Deep dependencies"
   - Quote: "Set (HashSet or BitString) of all links that are used by every single link."
   - Idea: Deep-dependency sets as HashSet or BitString.
50. **[konard/LinksPlatform#307](https://github.com/konard/LinksPlatform/issues/307)** - issue, created 2019-04-07, closed, by konard: "Compare HashSet<ulong> intersection and BitString intersection performance"
   - Quote: "Compare HashSet<ulong> intersection and BitString intersection performance"
   - Idea: Benchmark BitString intersection against HashSet.
   - Note: title only; see also https://github.com/linksplatform/Collections/issues/52
51. **[linksplatform/Collections#57](https://github.com/linksplatform/Collections/issues/57)** - issue, created 2019-12-02, open, by konard: "Think about mechanism of chaining multiple operations with BitString"
   - Quote: "Think about mechanism of chaining multiple operations with BitString"
   - Idea: Chained BitString operations; with Collections #19-#59, SIMD and performance work on BitString.
   - Note: title only
52. **[deep-foundation/deeplinks#28](https://github.com/deep-foundation/deeplinks/issues/28)** - issue, created 2021-12-05, open, by konard: "Dynamic PostgreSQL indexes"
   - Quote: "Collect indexes usage statistics"
   - Idea: Dynamic indexes built from usage statistics.
53. **[konard/LinksPlatform#4](https://github.com/konard/LinksPlatform/issues/4)** - issue, created 2015-09-10, closed, by konard: "Test separate storage of links values (Source, Target) and index (Binary Search Tree structure)"
   - Quote: "Test separate storage of links values (Source, Target) and index (Binary Search Tree structure)"
   - Idea: Split values from index storage (origin of the split layout).
   - Note: title only

### Journal, transactions, recovery, versioning (27)

54. **[konard/LinksPlatform#37](https://github.com/konard/LinksPlatform/issues/37)** - issue, created 2015-10-14, closed, by konard: "Durability / Transaction Log"
   - Quote: "Each transaction (line) is a transition betwen source link state and target link state."
   - Idea: Durability via a transaction log of transitions (before to after state); 0 before = create, 0 after = delete.
55. **[konard/LinksPlatform#39](https://github.com/konard/LinksPlatform/issues/39)** - issue, created 2015-10-14, open, by konard: "Consistency (Check / Recover)"
   - Quote: "Make a tool or integrate functions to check consistency and recover after database damage using transaction log data (transitions list)."
   - Idea: Consistency check and recovery from the log.
56. **[konard/LinksPlatform#40](https://github.com/konard/LinksPlatform/issues/40)** - issue, created 2015-10-14, closed, by konard: "Atomicity"
   - Quote: "Each transaction should be atomic ("all or none" transitions commited)."
   - Idea: Atomic groups of transitions.
57. **[konard/LinksPlatform#8](https://github.com/konard/LinksPlatform/issues/8)** - issue, created 2015-09-10, open, by konard: "Isolation / MultiThread support"
   - Quote: "Allow to execute multiple transactions at the same time."
   - Idea: Isolation and multi-threaded transactions.
58. **[konard/LinksPlatform#46](https://github.com/konard/LinksPlatform/issues/46)** - issue, created 2015-10-22, open, by konard: "Allow to push Links Data Snapshot to Transaction Log"
   - Quote: "Allow to push Links Data Snapshot to Transaction Log"
   - Idea: Snapshot written into the log.
   - Note: title only
59. **[konard/LinksPlatform#47](https://github.com/konard/LinksPlatform/issues/47)** - issue, created 2015-10-22, open, by konard: "Allow to compact (remove redundant transitions) a Transaction Log for achieving purposes"
   - Quote: "Allow to compact (remove redundant transitions) a Transaction Log for achieving purposes"
   - Idea: Log compaction for archiving.
   - Note: title only
60. **[konard/LinksPlatform#45](https://github.com/konard/LinksPlatform/issues/45)** - issue, created 2015-10-22, open, by konard: "A tool or function for converting Transaction Log file to Gexf Dynamic Graph"
   - Quote: "A tool or function for converting Transaction Log file to Gexf Dynamic Graph"
   - Idea: Export the log as a dynamic graph (visualise history).
   - Note: title only
61. **[konard/LinksPlatform#101](https://github.com/konard/LinksPlatform/issues/101)** - issue, created 2015-12-09, open, by konard: "Undo/Revert mechanism for transactions"
   - Quote: "It looks like there is a potential problem when we are doing revert for links pointing to itself."
   - Idea: Undo and revert of transactions, including holes left by deleted links.
62. **[konard/LinksPlatform#103](https://github.com/konard/LinksPlatform/issues/103)** - issue, created 2015-12-10, open, by konard: "Check saved transaction log timestamps are all in the past"
   - Quote: "Make consistency check (if opening transaction log for a second time)"
   - Idea: Check that log timestamps are all in the past.
63. **[konard/LinksPlatform#133](https://github.com/konard/LinksPlatform/issues/133)** - issue, created 2016-01-07, open, by konard: "Transactions log as lazy event loop"
   - Quote: "As event happens we notify about its time and position in log."
   - Idea: Transaction log as a lazy event loop.
64. **[konard/LinksPlatform#466](https://github.com/konard/LinksPlatform/issues/466)** - issue, created 2019-09-16, open, by konard: ""Create-only" or "no deletion" mode"
   - Quote: "It is possible to write all changes to the links state using only create operation."
   - Idea: Create-only (no deletion) mode that keeps the full history.
65. **[konard/LinksPlatform#474](https://github.com/konard/LinksPlatform/issues/474)** - issue, created 2019-10-03, open, by konard: "The database is a log, not a snapshot (approach)"
   - Quote: "If the database is small the only thing that should be written to disk is the log. The snapshot of the last version of the database can exist only in memory."
   - Idea: The database is a log; the snapshot lives only in memory.
66. **[konard/LinksPlatform#603](https://github.com/konard/LinksPlatform/issues/603)** - issue, created 2021-08-02, open, by konard: "Copy on write or Write to a copy principle"
   - Quote: "Database have two copies of the data. Only one copy is writable, the over one are readable with multiple threads."
   - Idea: Copy-on-write, or write to a copy, with pointer swap after commit.
67. **[konard/LinksPlatform#594](https://github.com/konard/LinksPlatform/issues/594)** - issue, created 2021-07-06, open, by konard: "Use StorJ as distributed transaction log storage"
   - Quote: "Use StorJ as distributed transaction log storage"
   - Idea: Distributed log storage.
   - Note: title only
68. **[konard/LinksPlatform#647](https://github.com/konard/LinksPlatform/issues/647)** - issue, created 2022-05-06, open, by konard: "LinksChain that can be stored inside associative storage"
   - Quote: "it will be possible to transform a range of links into links chain that can be functionally the same as blockchain."
   - Idea: LinksChain: a hash chain over ranges of links.
69. **[konard/LinksPlatform#435](https://github.com/konard/LinksPlatform/issues/435)** - issue, created 2019-08-15, open, by konard: "Idea for Links synchronization"
   - Quote: "The locking can be implemented on a per-link basis."
   - Idea: Per-link locking with a writer-only copy.
70. **[linksplatform/Data.Doublets#243](https://github.com/linksplatform/Data.Doublets/issues/243)** - issue, created 2021-09-22, open, by konard: "Implement transaction layer that stores all changes in memory"
   - Quote: "The idea is to store all transitions from source link state to target link state."
   - Idea: In-memory transaction layer of transitions, as in Neo4j.
   - Note: PR https://github.com/linksplatform/Data.Doublets/pull/442
71. **[linksplatform/Data.Doublets#250](https://github.com/linksplatform/Data.Doublets/issues/250)** - issue, created 2021-10-18, open, by konard: "Synchronization via automerged snapshots and changes sets"
   - Quote: "Each read operation should give a merged result of the saved database snapshot and the change set"
   - Idea: Snapshots plus change sets, merged on read.
72. **[linksplatform/Data.Doublets#208](https://github.com/linksplatform/Data.Doublets/issues/208)** - issue, created 2021-05-31, open, by konard: "Implement automatic data recovery"
   - Quote: "Link can exist if its references exist. All indexes are dropped and recreated."
   - Idea: Automatic recovery by rebuilding the indexes.
73. **[linksplatform/Data.Doublets#206](https://github.com/linksplatform/Data.Doublets/issues/206)** - issue, created 2021-05-30, open, by konard: "Shared access to the database file by two or more different processes"
   - Quote: "So it should be possible to access the database from different processes to use it as a medium for synchronization."
   - Idea: Shared multi-process access with a lock stored in the DB file.
   - Note: PR https://github.com/linksplatform/Data.Doublets/pull/459
74. **[link-foundation/link-cli#94](https://github.com/link-foundation/link-cli/issues/94)** - issue, created 2026-05-20, closed, by konard: "Optional transactions and version control layers"
   - Quote: "That log itself is also doublets storage, so it should be decorator on top of at least two links data stores - one for the data itself (or decorator on top of named links decorator), and one for the transactions and transitions."
   - Idea: Reversible transactions decorator whose log is itself a doublets store; infinite, chunked or limited log; sync or async apply; VersionControl decorator for time travel and branches (one transactions file per branch).
75. **[link-foundation/link-cli#98](https://github.com/link-foundation/link-cli/issues/98)** - issue, created 2026-08-29, closed, by konard: "Make the Rust library reusable as a doublets-backed transactional store, and update all dependencies in all languages"
   - Quote: "back `LinkStorage`/`NamedTypesDecorator` with `doublets` (`unit::Store` over a file-mapped memory), so the transactions layer composes over a real doublets database"
   - Idea: A reusable doublets-backed transactional store (persistent store plus transaction log) for downstream apps.
76. **[link-foundation/link-cli#129](https://github.com/link-foundation/link-cli/issues/129)** - issue, created 2026-10-21, open, by konard: "clink --import is not atomic: a malformed line exits 1 but keeps the links created from earlier lines"
   - Quote: "The import is all or nothing: on error the database is unchanged"
   - Idea: Atomic import.
77. **[linksplatform/doublets-rs#61](https://github.com/linksplatform/doublets-rs/issues/61)** - issue, created 2026-08-29, closed, by konard: "No transactions layer: C# has UInt64LinksTransactionsLayer, Rust has nothing"
   - Quote: "a durable transition log, so an interrupted process can be recovered rather than left half-written;"
   - Idea: Rust transactions layer with a durable transition log and replay.
   - Note: implemented by https://github.com/linksplatform/doublets-rs/pull/71 (merged): FileTransitionLog with checksummed records; a torn final record is discarded on reopen
78. **[linksplatform/Crawler#10](https://github.com/linksplatform/Crawler/issues/10)** - issue, created 2016-03-14, open, by konard: "Optimize total size of links storage and limit possible damage"
   - Quote: "- [ ] Turn on transaction log."
   - Idea: Per-site file split plus transaction log to limit damage.
79. **[uselessgoddess/molt#43](https://github.com/uselessgoddess/molt/pull/43)** - pr, created 2026-07-24, open, by konard: "Stage 3: add crash-consistent writable filesystem"
   - Quote: "introduce format version 2 with a typed, bounded three-bank journal and two-flush generation commits"
   - Idea: Crash-consistent writable FS: three-bank journal, generation commits (log write, flush, alternate superblock with generation+1 and log CRC, flush), replay of the newest valid generation on mount.
   - Note: not links-based: molt is uselessgoddess's research OS; PRs were opened under konard's account by the AI solver. Related molt PRs: #32 (read-only MoltROFS with checksummed regions and extents with holes), #45 (copy-on-write B+tree FS with atomic root publishing), #63 (extent tree)
80. **[link-assistant/formal-ai](https://github.com/link-assistant/formal-ai/blob/main/docs/associative-tech-stack.md)** - doc (docs/associative-tech-stack.md), created 2026-07-26, n/a, by n/a (doc file; first commit 2026-07-26): "Associative technology stack"
   - Quote: "`rust/src/link_store.rs` uses link-cli's file-mapped `DoubletsStorage` and `GenericTransactionsDecorator` directly: server writes are committed with an fsynced recovery log, explicit rollback restores both the native graph and its in-memory projection, and an interrupted projection is repaired from the atomic `.lino` source on reopen."
   - Idea: Shared transactional layer for an embedded links DB: file-mapped doublets plus a transactions decorator with an fsynced recovery log and rollback, and a readable .lino projection that is repaired on reopen.
   - Note: documentation file, not an issue; found while reading link-assistant/router#148, which cites it

### Memory and storage layout (united/split, headers, allocation, resizing, formats) (47)

81. **[konard/LinksPlatform#74](https://github.com/konard/LinksPlatform/issues/74)** - issue, created 2015-11-07, open, by konard: "Compare performance of storing all link fields in one file vs one field per file"
   - Quote: "Compare performance of storing all link fields in one file vs one field per file"
   - Idea: Compare one file against one file per field.
   - Note: title only; PR https://github.com/konard/LinksPlatform/pull/751
82. **[konard/LinksPlatform#48](https://github.com/konard/LinksPlatform/issues/48)** - issue, created 2015-10-22, open, by konard: "Small Links Data stores, allow use on target platform (64 bit) a lower bit configurations (eg. 32 bit, 16 bit, 8 bit)"
   - Quote: "Small Links Data stores, allow use on target platform (64 bit) a lower bit configurations (eg. 32 bit, 16 bit, 8 bit)"
   - Idea: Smaller address widths (8, 16, 32 bit).
   - Note: title only
83. **[konard/LinksPlatform#208](https://github.com/konard/LinksPlatform/issues/208)** - issue, created 2016-07-21, open, by konard: "Manage Links memory using ranges list instead of just linked list"
   - Quote: "Manage Links memory using ranges list instead of just linked list"
   - Idea: Free-space management as a list of ranges.
   - Note: title only
84. **[konard/LinksPlatform#207](https://github.com/konard/LinksPlatform/issues/207)** - issue, created 2016-07-21, open, by konard: "Random access links API"
   - Quote: "An ability to create a link in any desired location, and if this location already occupied then replace."
   - Idea: Random-access, stackless allocation API (best with #208).
85. **[konard/LinksPlatform#209](https://github.com/konard/LinksPlatform/issues/209)** - issue, created 2016-07-21, open, by konard: "Make Links parts number (2 for pairs, 3 for triples) changable (as setting or option)"
   - Quote: "Make Links parts number (2 for pairs, 3 for triples) changable (as setting or option)"
   - Idea: Configurable number of link parts.
86. **[konard/LinksPlatform#219](https://github.com/konard/LinksPlatform/issues/219)** - issue, created 2016-08-23, open, by konard: "Compare Pointers and Indicies implementations"
   - Quote: "The actual use case for Pointers is an extreme optimization when even small add operation or use of virtual memory counts."
   - Idea: Pointers vs indices.
87. **[konard/LinksPlatform#22](https://github.com/konard/LinksPlatform/issues/22)** - issue, created 2015-09-28, open, by konard: "Try to store startup information (such as mapped links) inside the very first link of database"
   - Quote: "Try to store startup information (such as mapped links) inside the very first link of database"
   - Idea: Header in the first link.
   - Note: title only
88. **[konard/LinksPlatform#465](https://github.com/konard/LinksPlatform/issues/465)** - issue, created 2019-09-16, open, by konard: "It is possible to not physically store the "point"-links"
   - Quote: "It is enough to just keep track of a number of such links and to reserve some range of addresses to refer to by other links."
   - Idea: Virtual point links.
89. **[konard/LinksPlatform#475](https://github.com/konard/LinksPlatform/issues/475)** - issue, created 2019-10-03, open, by konard: "Using small links addresses to store links storage snapshot in compact form"
   - Quote: "First, we reorder all the links by the number of usages."
   - Idea: Compact snapshot with variable address width blocks.
90. **[konard/LinksPlatform#591](https://github.com/konard/LinksPlatform/issues/591)** - issue, created 2021-07-01, open, by konard: "Editable array with infinite length"
   - Quote: "Each the change is required the updated range of values is written to the end of the array."
   - Idea: Append-only editable array of ranges.
91. **[konard/LinksPlatform#664](https://github.com/konard/LinksPlatform/issues/664)** - issue, created 2023-03-10, open, by konard: "Sectorized lists"
   - Quote: "This way we can get store lists each of element of with is system page sized array."
   - Idea: Sectorized lists of page-sized blocks.
92. **[konard/LinksPlatform#430](https://github.com/konard/LinksPlatform/issues/430)** - issue, created 2019-07-28, open, by konard: "Make LinksOptions and LinksFactory"
   - Quote: "This can be used to choose the correct combination of decorators."
   - Idea: LinksOptions and LinksFactory to compose storage at runtime.
93. **[konard/LinksPlatform#187](https://github.com/konard/LinksPlatform/issues/187)** - issue, created 2016-04-29, open, by konard: "Flexibility (functionality) and maximum performance at the same time for ILinks"
   - Quote: "Performance <-> Features."
   - Idea: Flexibility and maximum performance at the same time (precompiled variants).
94. **[konard/LinksPlatform#17](https://github.com/konard/LinksPlatform/issues/17)** - issue, created 2015-09-21, open, by konard: "Test situation when no more memory available on disk"
   - Quote: "Test situation when no more memory available on disk"
   - Idea: Out-of-disk behaviour.
   - Note: title only
95. **[konard/LinksPlatform#36](https://github.com/konard/LinksPlatform/issues/36)** - issue, created 2015-10-12, open, by konard: "Pack / Unpack (Backup, Restore)"
   - Quote: "Pack, backup - create a compact copy of database, using only minimum required data"
   - Idea: Pack and unpack format.
96. **[konard/LinksPlatform#29](https://github.com/konard/LinksPlatform/issues/29)** - issue, created 2015-10-03, open, by konard: "Links data structure as a binary data protocal (with optional compression using variable length numbers/sequences) (alternative to JSON, XML, ...)"
   - Quote: "Variable length numbers can be used"
   - Idea: Binary links protocol with variable-length numbers.
97. **[konard/LinksPlatform#194](https://github.com/konard/LinksPlatform/issues/194)** - issue, created 2016-05-26, open, by konard: "Universal Links String file format"
   - Quote: "One format that can store or transfer links of any size."
   - Idea: Universal Links String format.
98. **[konard/LinksPlatform#320](https://github.com/konard/LinksPlatform/issues/320)** - issue, created 2019-04-20, open, by konard: "Binary Sequences file format"
   - Quote: "This format have to contain two main sections: links section for duplicate data and sequences section for unique uncompressable sequences."
   - Idea: Binary sequences file format.
99. **[konard/LinksPlatform#645](https://github.com/konard/LinksPlatform/issues/645)** - issue, created 2022-04-29, open, by konard: "Store mutable data in RAM, and immutable data on Disk"
   - Quote: "States S(1) … S(n - 1) can be used as an archive and can be stored on Disk."
   - Idea: Mutable data in RAM, immutable data on disk.
100. **[konard/LinksPlatform#599](https://github.com/konard/LinksPlatform/issues/599)** - issue, created 2021-07-29, open, by konard: "Each table as a virtual links structure"
   - Quote: "Each row in additional table can be represented as virtual links structure."
   - Idea: Tables as virtual links.
101. **[konard/LinksPlatform#85](https://github.com/konard/LinksPlatform/issues/85)** - issue, created 2015-11-25, open, by konard: "Virtual links"
   - Quote: "Virtual links can be used to access additional or computed information that always applicable to any link"
   - Idea: Virtual or computed links.
102. **[konard/LinksPlatform#545](https://github.com/konard/LinksPlatform/issues/545)** - issue, created 2020-05-13, open, by konard: "Base vs optional data for links"
   - Quote: "Base data is the data without which the link will cease to exist."
   - Idea: Base vs optional data.
103. **[konard/LinksPlatform#199](https://github.com/konard/LinksPlatform/issues/199)** - issue, created 2016-06-10, open, by konard: "Try Array/List memory, that uses only managed memory"
   - Quote: "This will allow to run Links at any restricted environment of .NET."
   - Idea: Managed-memory-only backend.
104. **[konard/LinksPlatform#7](https://github.com/konard/LinksPlatform/issues/7)** - issue, created 2015-09-10, open, by konard: "Build example that is able to run on BareMetal OS"
   - Quote: "Also the Core.dll can be portable enough to run on BareMetal OS, but dependency on mmap (Memory Mapped Files) source be reconsidered."
   - Idea: Run on BareMetal OS; reconsider mmap.
105. **[linksplatform/Data.Doublets#179](https://github.com/linksplatform/Data.Doublets/issues/179)** - issue, created 2020-12-12, open, by konard: "Make a header for split memory implementation"
   - Quote: "The header should contain enough information to recreate index file it it is lost or dropped, or only data file was copied between storage devices."
   - Idea: Split-layout header (format version, allocated links, committed links) so the index file can be rebuilt.
   - Note: comment of 2021-01-15 adds committed-links counter; PR https://github.com/linksplatform/Data.Doublets/pull/473
106. **[linksplatform/Data.Doublets#172](https://github.com/linksplatform/Data.Doublets/issues/172)** - issue, created 2020-08-22, open, by konard: "For databases that do not delete data (all changes archived) automatic gaps filling feature can be optional (no need to check at runtime at allow)"
   - Quote: "For databases that do not delete data (all changes archived) automatic gaps filling feature can be optional"
   - Idea: Optional gap filling for append-only DBs.
107. **[linksplatform/Data.Doublets#131](https://github.com/linksplatform/Data.Doublets/issues/131)** - issue, created 2020-01-31, open, by konard: "Should we allocate 75% of available RAM or drive space at startup to reduce amount of reallocations?"
   - Quote: "Should we allocate 75% of available RAM or drive space at startup to reduce amount of reallocations?"
   - Idea: Preallocate most of the RAM or drive.
   - Note: title only; PR https://github.com/linksplatform/Data.Doublets/pull/488
108. **[linksplatform/Data.Doublets#372](https://github.com/linksplatform/Data.Doublets/issues/372)** - issue, created 2023-03-23, open, by konard: "Storage resize steps should be in links units instead of bytes."
   - Quote: "Storage resize steps should be in links units instead of bytes."
   - Idea: Resize in link units.
   - Note: title only
109. **[linksplatform/Data.Doublets#356](https://github.com/linksplatform/Data.Doublets/issues/356)** - issue, created 2022-07-11, open, by uselessgoddess: "Maybe rework header logic to contain metadata in `size_type`"
   - Quote: "Then it would be possible to store any number of links of any type."
   - Idea: Header metadata in size_type.
110. **[linksplatform/Data.Doublets#317](https://github.com/linksplatform/Data.Doublets/issues/317)** - issue, created 2022-03-08, closed, by uselessgoddess: "Don't shred files"
   - Quote: "After shrink allocated memory in `FileMappedMem`, file also shrink."
   - Idea: Do not shrink files.
111. **[linksplatform/Data.Doublets#155](https://github.com/linksplatform/Data.Doublets/issues/155)** - issue, created 2020-03-21, open, by konard: "Should we enable external references by default in all memory adapters?"
   - Quote: "Should we enable external references by default in all memory adapters?"
   - Idea: External references by default.
   - Note: title only
112. **[linksplatform/Data.Doublets#168](https://github.com/linksplatform/Data.Doublets/issues/168)** - issue, created 2020-07-26, closed, by konard: "Individual trees in Split memory adapter can use only one field for comparison of tree node's keys"
   - Quote: "Individual trees in Split memory adapter can use only one field for comparison of tree node's keys"
   - Idea: Split-layout tree keys.
   - Note: title only
113. **[linksplatform/Memory#48](https://github.com/linksplatform/Memory/issues/48)** - issue, created 2020-12-27, open, by konard: "Add non-resizable memory implementations"
   - Quote: "For example to allocate all ram or whole drive of storage at once."
   - Idea: Non-resizable memory over the whole RAM or drive.
114. **[linksplatform/Memory#21](https://github.com/linksplatform/Memory/issues/21)** - issue, created 2019-09-03, open, by konard: "Add DistributedDirectMemory"
   - Quote: "* Multi drive memory distribution"
   - Idea: DistributedDirectMemory across drives and machines.
   - Note: PR https://github.com/linksplatform/Memory/pull/84
115. **[linksplatform/Memory#11](https://github.com/linksplatform/Memory/issues/11)** - issue, created 2019-07-27, open, by konard: "Add VirtualResizableDirectMemory, that uses virtual memory API"
   - Quote: "Add VirtualResizableDirectMemory, that uses virtual memory API"
   - Idea: Virtual memory API backend.
   - Note: title only
116. **[linksplatform/Memory#24](https://github.com/linksplatform/Memory/issues/24)** - issue, created 2019-09-15, closed, by konard: "Should depend on actual virtual memory page size"
   - Quote: "Should depend on actual virtual memory page size"
   - Idea: Page-size-aware growth.
   - Note: title only
117. **[linksplatform/Data#77](https://github.com/linksplatform/Data/issues/77)** - issue, created 2021-10-27, open, by konard: "MapReduce multithread combined storage"
   - Quote: "Then we can create separate file or allocate separate section for each new N links."
   - Idea: Sectioned storage (one section or file per N links, one thread per section).
118. **[linksplatform/Data.Triplets.Kernel#11](https://github.com/linksplatform/Data.Triplets.Kernel/issues/11)** - issue, created 2020-10-23, open, by app/: "Implement file consistency check (validation)"
   - Quote: "TODO: Varidate all freed link (holes). (slower)"
   - Idea: File consistency check of mapped links, holes and all links.
119. **[linksplatform/mem-rs#36](https://github.com/linksplatform/mem-rs/issues/36)** - issue, created 2026-10-05, open, by konard: "RawMem::grow_filled and grow_zeroed overwrite data already in a FileMapped file, so a reopened doublets store is empty"
   - Quote: "RawMem::grow_filled and grow_zeroed overwrite data already in a FileMapped file, so a reopened doublets store is empty"
   - Idea: File-mapped growth bug (persistence pitfall).
   - Note: title
120. **[linksplatform/mem-rs#26](https://github.com/linksplatform/mem-rs/issues/26)** - issue, created 2026-01-13, closed, by konard: "In addition to sync mmap access also support async memory access as efficiently as possible, add bechmarks"
   - Quote: "In addition to sync mmap access also support async memory access as efficiently as possible, add bechmarks"
   - Idea: Async memory access.
   - Note: title only; PR https://github.com/linksplatform/mem-rs/pull/27
121. **[linksplatform/doublets-rs#69](https://github.com/linksplatform/doublets-rs/issues/69)** - issue, created 2026-10-08, open, by konard: "We need to double check we use exact memory layouts for Link struct"
   - Quote: "So we control exactly how it maps to memory and disk."
   - Idea: Exact repr(C) link layout.
122. **[linksplatform/Comparisons.SQLiteVSDoublets#29](https://github.com/linksplatform/Comparisons.SQLiteVSDoublets/issues/29)** - issue, created 2020-03-22, closed, by konard: "Check that the size of database for Split memory adapter calculated correctly"
   - Quote: "Check that the size of database for Split memory adapter calculated correctly"
   - Idea: Split DB size accounting.
   - Note: title only
123. **[konard/LinksPlatform](https://github.com/konard/LinksPlatform/wiki)** - wiki, created wiki clone, n/a, by konard: "Home (wiki page)"
   - Quote: "Links storage size in bytes depending on the number of links"
   - Idea: Spreadsheet of storage size by link count.
   - Note: wiki Home.md link
124. **[gist konard/9c8115c40f47edd8b1b8](https://gist.github.com/konard/9c8115c40f47edd8b1b8)** - gist, created 2014-11-25, n/a, by konard: "Links Sequences Triggers.md"
   - Quote: "Реализация Memory под Virtual Memory c возможностью изменения размера, но без хранения на диске."
   - Translation: "Implementation of Memory on top of Virtual Memory with the ability to change size, but without storing on disk."
   - Idea: Earliest recorded note on the memory layer: a resizable memory implementation on top of virtual memory, without disk persistence (in-memory variant of the links store).
   - Note: gist 'Links Sequences Triggers.md', 2014-11-25; the rest of the gist is about sequences, triggers and rules
125. **[link-assistant/router#357](https://github.com/link-assistant/router/issues/357)** - issue, created 2026-08-28, closed, by konard: "The doublets store is opened, rebuilt and dropped on every access: hold one instance for the process lifetime behind an RwLock"
   - Quote: "`associative::write_binary` and `read_binary` each open the file, memory-map it and construct a fresh `unit::Store` on **every** call, then drop it."
   - Idea: A file-backed doublets store must be long-lived (one instance per process behind a reader-writer lock); reopening and rebuilding per access made reads take 8-14 s versus a 0.25 s disk write.
   - Note: fixed by link-assistant/router PR #367
126. **[link-assistant/router#557](https://github.com/link-assistant/router/issues/557)** - issue, created 2026-09-09, closed, by konard: "Token store capacity panics the issuing request after persisting an uncertain token"
   - Quote: "A long-lived Router token store eventually reaches the fixed doublets mapping capacity. The next client-token issuance panics its Tokio worker"
   - Idea: Field evidence for a growable store: a fixed 64 MB doublets mapping filled up and panicked in the size-balanced tree; the store must grow or compact, or fail with a structured error, never panic.
127. **[link-assistant/router#374](https://github.com/link-assistant/router/issues/374)** - issue, created 2026-08-30, closed, by konard: "v0.125.0 cannot write to a token store from a previous release: every put fails with 'doublets schema contains an invalid point', yet issue still returns a token"
   - Quote: "After the persistent binary-store backend changed, a token store written by an earlier release could still be read but failed on every subsequent write."
   - Idea: On-disk format versioning: migrate a legacy binary store before accepting writes, or fail at startup; never report success unless persistence succeeded.

### Cross-platform drivers, OS and host file-system integration (7)

128. **[konard/LinksPlatform#215](https://github.com/konard/LinksPlatform/issues/215)** - issue, created 2016-08-04, open, by konard: "Links Operation System"
   - Quote: "Prototype: http://www.returninfinity.com/baremetal.html (BareMetal OS)"
   - Idea: Links OS.
129. **[deep-foundation/deep-packages#31](https://github.com/deep-foundation/deep-packages/issues/31)** - issue, created 2023-11-29, open, by konard: "Sync files from Deep with folder on disk in any operating system"
   - Quote: "Users, Spaces, Packages should be interpreted as folders."
   - Idea: Two-way sync of Deep links with a folder on disk in any OS.
   - Note: mirrored as https://github.com/link-foundation/deep-packages/issues/101; references https://github.com/deep-foundation/dev/issues/125 which no longer resolves
130. **[link-foundation/deep-packages#114](https://github.com/link-foundation/deep-packages/issues/114)** - issue, created 2024-09-04, open, by konard: "Sync link with file on client system"
   - Quote: "We want to sync its changes to our file-system and sync changes of the client file to deep"
   - Idea: Sync one link with one client file.
   - Note: fork of deep-foundation/deep-packages#16
131. **[deep-foundation/deep-packages#30](https://github.com/deep-foundation/deep-packages/issues/30)** - issue, created 2023-08-08, open, by SonnyRoot: "Synchronization of Deep and VS Code"
   - Quote: "where all the handler files are synchronized with the file system and with the Deep"
   - Idea: Deep and VS Code sync through the file system.
   - Note: mirrored as https://github.com/link-foundation/deep-packages/issues/126
132. **[deep-foundation/deep](https://github.com/deep-foundation/deep/blob/83df56142a8ce06cc91a03358bca58be24414157/README.md)** - doc (README.md), created n/a, n/a, by n/a (code search): "README.md roadmap"
   - Quote: "- [ ] File system watch/sync"
   - Idea: Roadmap item: file-system watch and sync.
   - Note: code search hit; README file, not an issue
133. **[deep-foundation/hasura#13](https://github.com/deep-foundation/hasura/issues/13)** - issue, created 2023-10-02, open, by Archer-Lotos: "Enhanced Flexibility and Cost Optimization for MinIO and Amazon S3"
   - Quote: "We propose introducing a feature that empowers users to choose the storage location for individual files within MinIO and Amazon S3."
   - Idea: Per-file storage location.
   - Note: mirrored as https://github.com/link-foundation/hasura/issues/6
134. **[konard/LinksPlatform#601](https://github.com/konard/LinksPlatform/issues/601)** - issue, created 2021-07-29, open, by konard: "Run Links-based storage in shadow mode to test it works correctly as a replacement for any other storage type"
   - Quote: "Links work as slave-only replica that only replicates data changes, but is not used as a primary storage."
   - Idea: Shadow-mode replica.

### Visualization (3)

135. **[konard/LinksPlatform#617](https://github.com/konard/LinksPlatform/issues/617)** - issue, created 2021-08-30, open, by konard: "Visualization of memory pages accessi"
   - Quote: "It should help understand how the memory is accessed to find new ways to optimize memory structure of the database."
   - Idea: Visualise memory page access.
   - Note: PR https://github.com/konard/LinksPlatform/pull/1044
136. **[konard/links-visuals#23](https://github.com/konard/links-visuals/issues/23)** - issue, created 2026-03-17, closed, by konard: "Take best practices from blueprint.html, to make grid.html"
   - Quote: "By default links should fill square grid"
   - Idea: Grid of links in the links-visuals blueprint style (GitHub Pages).
   - Note: not FS-specific
137. **[uselessgoddess/pngfs-priv#2](https://github.com/uselessgoddess/pngfs-priv/pull/2)** - pr, created 2026-08-18, open, by konard: "pngfs: mountable encrypted filesystem hidden in PNG and JPEG (Rust + WASM)"
   - Quote: "A Rust core compiled to WebAssembly that hides an encrypted, deniable virtual filesystem inside ordinary images — **PNG and JPEG** — plus a CLI and a 100 % client-side web app (deployable to GitHub Pages)."
   - Idea: A Rust FS core compiled to WASM, with a client-side GitHub Pages app that mounts and edits the volume in the browser (same pattern as a browser disk visualizer).
   - Note: not links-based; AI-solver PR under konard's account in uselessgoddess's repo

### Other related storage ideas (dual text/binary stores, backends, backups) (17)

138. **[link-foundation/start#42](https://github.com/link-foundation/start/issues/42)** - issue, created 2025-12-31, closed, by konard: "Use application folder and links notation to store information about each called command"
   - Quote: "We need to support dual storage - binary using database and text files using links notation parser and formatter"
   - Idea: Dual storage: a binary doublets DB plus text LiNo files.
139. **[konard/vietnam-accomodation-search#4](https://github.com/konard/vietnam-accomodation-search/issues/4)** - issue, created 2026-09-22, closed, by konard: "Add transactional link-cli binary storage alongside canonical Links Notation"
   - Quote: "atomically writes `.lino` files but has no transactional binary mirror, recovery log, or link query path."
   - Idea: Transactional binary link-cli store alongside the LiNo text files.
140. **[link-foundation/links-logs-vault#1](https://github.com/link-foundation/links-logs-vault/issues/1)** - issue, created 2026-06-15, closed, by konard: "Initial 0.0.1 version"
   - Quote: "So we should be able to use both text and binary stores."
   - Idea: Logs vault on text and binary links stores.
141. **[link-foundation/links-queue#13](https://github.com/link-foundation/links-queue/issues/13)** - issue, created 2026-01-10, closed, by konard: "[Phase 3] Define StorageBackend interface/trait (API Contract)"
   - Quote: "This abstraction allows switching between memory, link-cli, and custom backends via configuration."
   - Idea: Pluggable storage backends.
142. **[link-foundation/links-notation#325](https://github.com/link-foundation/links-notation/issues/325)** - issue, created 2026-10-05, closed, by konard: "Proposal: one binary links notation shared by all implementations"
   - Quote: "This proposes that links-notation own a single **binary links notation** spec"
   - Idea: One shared binary links notation.
143. **[link-foundation/meta-language#54](https://github.com/link-foundation/meta-language/issues/54)** - issue, created 2026-06-10, closed, by konard: "Binary doublets storage backend (doublets-rs) behind a storage trait"
   - Quote: "No binary links storage exists today"
   - Idea: doublets-rs storage backend behind a trait.
144. **[link-foundation/meta-language#14](https://github.com/link-foundation/meta-language/issues/14)** - issue, created 2026-06-06, closed, by konard: "Persistent immutable snapshots (green/red model; optional doublets substrate)"
   - Quote: "Issue #1 §3.5 requires immutable snapshots that are "persistent, structurally shared.""
   - Idea: Persistent, structurally shared snapshots.
145. **[link-foundation/meta-sovereign#6](https://github.com/link-foundation/meta-sovereign/issues/6)** - issue, created 2026-05-02, closed, by konard: "Improvements and hardening system from data loss or unauthorized access"
   - Quote: "so our unified links data store is always friendly for backups. Yet by default it should be encrypt"
   - Idea: Unified links store with backups and encryption.
146. **[konard/LinksPlatform#227](https://github.com/konard/LinksPlatform/issues/227)** - issue, created 2016-10-17, open, by konard: "Any/All Account/Profile Sync/Backup as way to own your data"
   - Quote: "Allow users to have a local copy of everything they ever put in the internet."
   - Idea: Own your data.
147. **[konard/LinksPlatform#303](https://github.com/konard/LinksPlatform/issues/303)** - issue, created 2019-04-04, open, by konard: "Global data dependency graph or global data version control"
   - Quote: "Global data dependency graph or global data version control"
   - Idea: Global version control of fragments.
148. **[konard/LinksPlatform#703](https://github.com/konard/LinksPlatform/issues/703)** - issue, created 2025-10-16, open, by konard: "Video Metadata Organization using Links"
   - Quote: "Use Links Platform for structuring video metadata with advanced features."
   - Idea: Video metadata in links.
149. **[konard/LinksPlatform#700](https://github.com/konard/LinksPlatform/issues/700)** - issue, created 2025-10-16, open, by konard: "Text Editor with Meta-Information Synchronization"
   - Quote: "Move annotations and color highlights into separate meta-information layer"
   - Idea: Text editor with a meta-information layer.
150. **[konard/cbr-key-rate-monitor#4](https://github.com/konard/cbr-key-rate-monitor/issues/4)** - issue, created 2025-08-08, open, by konard: "Use clink (link-cli) or Data.Doublets as database engine"
   - Quote: "Use clink (link-cli) or Data.Doublets as database engine"
   - Idea: Doublets as an app database.
   - Note: title only
151. **[link-assistant/router#148](https://github.com/link-assistant/router/issues/148)** - issue, created 2026-08-13, closed, by konard: "Storage does not use the associative stack: no doublets, and tokens.lino is hand-built string concatenation"
   - Quote: "Per the associative technology stack, persistent data should live in the associative store — links notation plus binary doublets — rather than in ad-hoc formats."
   - Idea: Application storage rule: binary doublets store (doublets-rs on platform-mem) as the persistent backend, with Links Notation as the readable import/export projection.
   - Note: the stack is described in link-assistant/formal-ai docs/associative-tech-stack.md
152. **[link-assistant/router#235](https://github.com/link-assistant/router/issues/235)** - issue, created 2026-08-20, closed, by konard: "Store all router state as links notation: the token store already does, the request log and three smaller stores still use JSON"
   - Quote: "make links notation the storage format for **all** router-owned state, in a human-readable form"
   - Idea: Dual store in practice: tokens.lino (text projection) next to tokens.bin (64 MB binary link store) plus tokens.lock and tokens.transaction.lock; extend it to all application state.
153. **[AKMAxelerator/CorrelationCenterConcept#5](https://github.com/AKMAxelerator/CorrelationCenterConcept/issues/5)** - issue, created 2015-09-10, open, by konard: "Implement storage using Links database"
   - Quote: "Implement storage using Links database"
   - Idea: Early (2015) proposal to use the Links database as the storage of another project.
   - Note: title only; the body is just a link to konard/LinksPlatform
154. **[deep-foundation/deep-packages#69](https://github.com/deep-foundation/deep-packages/issues/69)** - issue, created 2024-01-22, open, by konard: "A package that loads doublets from LinksPlatform.Data.Doublets to Deep"
   - Quote: "To achieve this task, it is required to create `Doublet` type in `doublets` package."
   - Idea: Load doublets from LinksPlatform Data.Doublets into Deep through a Doublet type (bridge between the doublets store and Deep).
   - Note: mirrored as link-foundation/deep-packages#64

## Secondary sources: implementation and draft PRs

These PRs (mostly AI-generated in 2025-2026) implement or draft the ideas above. They are listed for traceability; the ideas are attributed to the issues.

| PR | state | created | title |
|---|---|---|---|
| [konard/LinksPlatform#866](https://github.com/konard/LinksPlatform/pull/866) | open | 2025-10-17 | [WIP] Links File System |
| [konard/LinksPlatform#863](https://github.com/konard/LinksPlatform/pull/863) | open | 2025-10-17 | Add raw drive storage design and implementation for issue #213 |
| [konard/LinksPlatform#834](https://github.com/konard/LinksPlatform/pull/834) | open | 2025-10-16 | Tag File System Implementation |
| [konard/LinksPlatform#807](https://github.com/konard/LinksPlatform/pull/807) | open | 2025-10-16 | Data importers: JSON and FileSystem |
| [konard/LinksPlatform#708](https://github.com/konard/LinksPlatform/pull/708) | open | 2025-10-16 | [WIP] Allow store raw/binary data inside links database |
| [konard/LinksPlatform#1023](https://github.com/konard/LinksPlatform/pull/1023) | open | 2025-10-17 | [WIP] Links and Binary data |
| [konard/LinksPlatform#1011](https://github.com/konard/LinksPlatform/pull/1011) | open | 2025-10-17 | Implement bit string variations with bitmap and sector-based storage |
| [konard/LinksPlatform#1012](https://github.com/konard/LinksPlatform/pull/1012) | open | 2025-10-17 | Implement sectorized list data structure |
| [konard/LinksPlatform#986](https://github.com/konard/LinksPlatform/pull/986) | open | 2025-10-17 | Add BitString-based indexing layer for space-efficient sequence storage |
| [konard/LinksPlatform#1044](https://github.com/konard/LinksPlatform/pull/1044) | open | 2025-10-26 | Add memory page access visualization tool |
| [konard/LinksPlatform#1058](https://github.com/konard/LinksPlatform/pull/1058) | open | 2025-10-26 | Implement hybrid storage architecture: RAM for mutable data, Disk for immutable data |
| [konard/LinksPlatform#1039](https://github.com/konard/LinksPlatform/pull/1039) | open | 2025-10-26 | Implement Copy-On-Write (Write-To-A-Copy) Database Principle |
| [konard/LinksPlatform#917](https://github.com/konard/LinksPlatform/pull/917) | open | 2025-10-17 | Log-based storage architecture for small databases |
| [konard/LinksPlatform#926](https://github.com/konard/LinksPlatform/pull/926) | open | 2025-10-17 | Add OS recovery system design based on Links Platform |
| [konard/LinksPlatform#751](https://github.com/konard/LinksPlatform/pull/751) | open | 2025-10-16 | Compare performance of storing all link fields in one file vs one field per file |
| [konard/LinksPlatform#733](https://github.com/konard/LinksPlatform/pull/733) | open | 2025-10-16 | [WIP] Small Links Data stores, allow use on target platform (64 bit) a lower bit configurations (eg. 32 bit, 16 bit, 8 bit) |
| [konard/LinksPlatform#904](https://github.com/konard/LinksPlatform/pull/904) | open | 2025-10-17 | Add LinksFactory and LinksOptions Design Specification |
| [konard/LinksPlatform#894](https://github.com/konard/LinksPlatform/pull/894) | open | 2025-10-17 | [WIP] Binary Sequences file format |
| [konard/LinksPlatform#846](https://github.com/konard/LinksPlatform/pull/846) | open | 2025-10-16 | Universal Links String file format |
| [konard/LinksPlatform#718](https://github.com/konard/LinksPlatform/pull/718) | open | 2025-10-16 | Binary Links Notation (BLN) - Specification and Examples |
| [linksplatform/Data.Doublets#511](https://github.com/linksplatform/Data.Doublets/pull/511) | open | 2025-10-30 | Allow treating links references as binary data |
| [linksplatform/Data.Doublets#473](https://github.com/linksplatform/Data.Doublets/pull/473) | open | 2025-09-13 | Add file format version and committed links counter to LinksHeader for split memory recovery |
| [linksplatform/Data.Doublets#400](https://github.com/linksplatform/Data.Doublets/pull/400) | open | 2025-09-11 | Support bitstring indexing as alternative or additional to trees |
| [linksplatform/Data.Doublets#399](https://github.com/linksplatform/Data.Doublets/pull/399) | open | 2025-09-11 | Implement bitstring approach for deleted links tracking - closes #398 |
| [linksplatform/Data.Doublets#442](https://github.com/linksplatform/Data.Doublets/pull/442) | open | 2025-09-13 | Implement Transaction Layer with Neo4j-inspired Model |
| [linksplatform/Data.Doublets#459](https://github.com/linksplatform/Data.Doublets/pull/459) | open | 2025-09-13 | Implement shared access to database file by multiple processes |
| [linksplatform/Data.Doublets#488](https://github.com/linksplatform/Data.Doublets/pull/488) | open | 2025-09-14 | Implement configurable memory pre-allocation for reduced reallocations |
| [linksplatform/Memory#84](https://github.com/linksplatform/Memory/pull/84) | open | 2025-09-14 | Add DistributedDirectMemory for multi-drive memory distribution |
| [linksplatform/mem-rs#27](https://github.com/linksplatform/mem-rs/pull/27) | merged | 2026-01-13 | Add async memory operations with AsyncFileMem (mmap + I/O thread) |
| [linksplatform/doublets-rs#71](https://github.com/linksplatform/doublets-rs/pull/71) | merged | 2026-10-08 | feat(transactions): add generic transactions and durable recovery |

## Reviewed and excluded

- Keyword hits that are about tooling, not storage design: link-foundation/box, disk-space-saviour, start, rust-web-box, use-m, command-stream (file-system built-ins), browser-commander (downloads), lino-arguments, gh-upload-log, konard/hh-questions, konard/vietnam-accomodation-search (except #4), konard/medical-neural-network-ml, konard/p-vs-np.
- Global `author:konard` hits in third-party repositories (AI-solver PRs opened under the konard account, e.g. dronedoc2025, godot-topdown-MVP, rumaster/2book-es, suenot/*, xlabtg/*, docker/moby issues): app-level file handling, not storage design. The exceptions kept above are uselessgoddess/molt and uselessgoddess/pngfs-priv.
- link-assistant/agent, claude-profiles, master-plan, web-capture, react-chat-ui and the remaining router hits: file-system access by CLIs and bots, not storage design.
- link-assistant/hive-mind and link-assistant/formal-ai issues: thousands of hits on `file`, `storage`, `journal`, `transaction`; reviewed via the dump grep, none describe a links file system or embedded-files DB.
- Template and CI repositories (`*-template`, workflow bumps, `chore(deps)`).
- deep-foundation discussions (org #7, #10, #13, #16, #29, #34; deep-memo-app #100 "Memory"; dev #27 "id for client usages"; dev #39 "DDD"): matched `file`, `storage`, `database`, `blob` or `хранилище`, but none is about a file system or embedded files.
- linksplatform/IO and Interfaces hits (`blob` in links, temporary files, translation PRs).
- konard/links-visuals is listed only once (VIS) because it defines the visual style of links, not a disk map.
