# Observations

Rendered by `harness observe` — do not edit.

Detector suite: `c-treesitter-v1` · findings: 31 · annotations: 2

## Units by risk

| unit | status | score | blockers | findings (confirmed/dismissed/uncertain) |
|---|---|---|---|---|
| u-blocksplitter-deflate-squeeze | pending | 51 | — | 16/3/1 |
| u-lz77 | pending | 28 | — | 4/0/0 |
| u-zopfli_bin | pending | 15 | — | 6/0/0 |
| u001-katajainen | verified | 13 | — | 3/1/0 |
| u-zopfli_lib | pending | 12 | — | 2/0/0 |
| u-cache | pending | 11 | — | 3/0/0 |
| u-gzip_container | pending | 11 | — | 2/0/0 |
| u-hash | pending | 11 | — | 3/0/0 |
| u-tree | pending | 11 | — | 2/1/0 |
| u-zlib_container | pending | 11 | — | 2/0/0 |
| u-util | pending | 5 | — | 2/0/0 |

## Findings

### f-2e2b0b2a732d07f1 — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/blocksplitter.c:36–36` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- typedef FindMinimumFun declares a function type: indirect calls are invisible to the scanner
- triage (confirm, high): typedef FindMinimumFun (blocksplitter.c:36) declares a bare function type used for callback dispatch; calls through it are invisible to the scanner's call graph, so unit dependency edges are incomplete and a Rust migration must redesign this as a closure/fn-trait boundary.

### f-524f9b6241de430a — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/blocksplitter.c:215–273` · severity: info · affects: u-blocksplitter-deflate-squeeze
- ZopfliBlockSplitLZ77: heap ownership crosses the function boundary (calls free)
- triage (confirm, high): ZopfliBlockSplitLZ77 grows the caller-provided splitpoints array via ZOPFLI_APPEND_DATA and frees intermediate arrays; the caller later frees storage the callee allocated — a classic who-frees convention that must become owned Vec returns in Rust.

### f-6101025093d0e416 — alloc `alloc-ownership` (dismissed — pending human review)

- location: `src/zopfli/blocksplitter.c:148–180` · severity: info · affects: u-blocksplitter-deflate-squeeze
- PrintBlockSplitPoints: heap ownership crosses the function boundary (calls free)
- triage (dismiss, medium): PrintBlockSplitPoints allocates and frees its text buffer locally; a contained alloc/free pair that translates to an owned String/Vec unchanged.

### f-9845f7ece303a22f — fn-pointer `function-pointer-arg` (confirmed)

- location: `src/zopfli/blocksplitter.c:244–244` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- function SplitCost passed as a value argument: forms an indirect call edge
- triage (confirm, medium): SplitCost passed by name into FindMinimum (blocksplitter.c:244) forms the indirect call edge the scanner cannot see; the migration must carry this edge explicitly.

### f-b41e5cfd2e935724 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/blocksplitter.c:275–320` · severity: info · affects: u-blocksplitter-deflate-squeeze
- ZopfliBlockSplit: heap ownership crosses the function boundary (calls free)
- triage (confirm, high): ZopfliBlockSplit allocates *splitpoints that the caller (ZopfliDeflatePart) later frees — cross-function ownership transfer by convention.

### f-f8c3c67711d9bf0a — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/blocksplitter.c:43–43` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- parameter of function or function-pointer type: indirect call edge invisible to scanner
- triage (confirm, high): FindMinimum (blocksplitter.c:43) takes a FindMinimumFun parameter plus a void* context — a type-erased callback pair. The void* context defeats type checking and the indirect call hides the SplitCost edge; Rust migration needs a generic or trait-object redesign.

### f-27b139fc4d706a24 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/cache.c:48–52` · severity: info · affects: u-cache
- ZopfliCleanCache: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): ZopfliCleanCache frees fields of a caller-owned struct — the init/clean destructor convention. Maps to Drop in Rust, but the migration must prove no path double-frees or uses-after-clean across the mixed C/Rust boundary.

### f-49c34fc3dddd0b6f — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/deflate.c:811–906` · severity: info · affects: u-blocksplitter-deflate-squeeze
- ZopfliDeflatePart: heap ownership crosses the function boundary (calls free)
- triage (confirm, high): ZopfliDeflatePart frees splitpoint arrays allocated inside ZopfliBlockSplit — allocation and deallocation live in different functions, ownership conveyed purely by convention.

### f-96564fa0448f6f91 — alloc `alloc-ownership` (dismissed — pending human review)

- location: `src/zopfli/deflate.c:434–518` · severity: info · affects: u-blocksplitter-deflate-squeeze
- OptimizeHuffmanForRle: heap ownership crosses the function boundary (calls free)
- triage (dismiss, medium): OptimizeHuffmanForRle allocates good_for_rle and frees it before returning; the alloc/free pair is fully contained in the function and maps directly to an owned Vec with no boundary-crossing ownership.

### f-ef881ef2b2212b24 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/deflate.c:105–249` · severity: info · affects: u-blocksplitter-deflate-squeeze
- EncodeTree: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): EncodeTree (deflate.c) pairs its local rle buffers correctly, but it appends into caller-owned output buffers via ZOPFLI_APPEND_DATA, so allocation of the caller's buffer happens inside the callee — ownership crosses the boundary through the macro.

### f-9faf4a821f87cdab — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/hash.c:75–89` · severity: info · affects: u-hash
- ZopfliCleanHash: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): ZopfliCleanHash is the destructor half of an alloc/clean convention on caller-owned ZopfliHash; same Drop-mapping caveats as the cache destructor.

### f-b1a4a8d9139e41ff — alloc `alloc-ownership` (dismissed — pending human review)

- location: `src/zopfli/katajainen.c:172–262` · severity: info · affects: u001-katajainen
- ZopfliLengthLimitedCodeLengths: heap ownership crosses the function boundary (calls free)
- triage (dismiss, high): ZopfliLengthLimitedCodeLengths allocates leaves/nodes/lists and frees all of them on every path including error returns; ownership never escapes. The verified u001 Rust migration already replaced this with a safe arena — evidence the pattern was contained.

### f-ecf08778de446ba1 — fn-pointer `function-pointer-arg` (confirmed)

- location: `src/zopfli/katajainen.c:232–232` · severity: medium · affects: u001-katajainen
- function LeafComparator passed as a value argument: forms an indirect call edge
- triage (confirm, high): LeafComparator passed to qsort (katajainen.c:232) is an indirect call the scanner cannot see, and the annotated comparator-UB finding (f-b51a4854969c64bd) lives in exactly this callback — comparator callbacks deserve confirmed hazard status.

### f-88193f54e8e06410 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/lz77.c:40–48` · severity: info · affects: u-lz77
- ZopfliCleanLZ77Store: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): ZopfliCleanLZ77Store frees caller-owned store internals — destructor-by-convention; Drop mapping with mixed-boundary double-free risk.

### f-bf94bb4017cc5914 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/lz77.c:235–242` · severity: info · affects: u-lz77
- ZopfliCleanBlockState: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): ZopfliCleanBlockState delegates to ZopfliCleanCache on an optionally-present lmc — conditional ownership (add_lmc flag decides who allocated), which types must make explicit in Rust.

### f-30f1a39668d72fec — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/squeeze.c:432–432` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- parameter of function or function-pointer type: indirect call edge invisible to scanner
- triage (confirm, medium): LZ77OptimalRun (squeeze.c:432) takes a CostModelFun* + void* context pair; the indirect cost-model call is invisible to the call graph and type-erased via void*.

### f-4e42c743c33e06b5 — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/squeeze.c:119–119` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- typedef CostModelFun declares a function type: indirect calls are invisible to the scanner
- triage (confirm, high): typedef CostModelFun (squeeze.c:119) is the cost-model callback type threaded through the optimal-parse pipeline with void* contexts; central indirection invisible to the call graph.

### f-62c48a1ec483e0ed — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/squeeze.c:220–220` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- parameter of function or function-pointer type: indirect call edge invisible to scanner
- triage (confirm, medium): GetBestLengths (squeeze.c:220) takes CostModelFun* + void* costcontext — type-erased indirect dispatch on the hot path.

### f-6ceab10def00fc3f — fn-pointer `function-pointer-arg` (confirmed)

- location: `src/zopfli/squeeze.c:554–554` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- function GetCostFixed passed as a value argument: forms an indirect call edge
- triage (confirm, medium): GetCostFixed passed by name (squeeze.c:554) into the optimal-parse run — indirect call edge to carry explicitly.

### f-74857d6e64ebd1b7 — fn-pointer `function-pointer-decl` (confirmed)

- location: `src/zopfli/squeeze.c:163–163` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- parameter of function or function-pointer type: indirect call edge invisible to scanner
- triage (confirm, medium): GetCostModelMinCost (squeeze.c:163) takes the CostModelFun* callback; same indirect-dispatch hazard.

### f-8864ac2364af4cdc — fn-pointer `function-pointer-arg` (confirmed)

- location: `src/zopfli/squeeze.c:490–490` · severity: medium · affects: u-blocksplitter-deflate-squeeze
- function GetCostStat passed as a value argument: forms an indirect call edge
- triage (confirm, medium): GetCostStat passed by name (squeeze.c:490) — same indirect call edge as GetCostFixed.

### f-be415b3538a377d6 — alloc `alloc-ownership` (uncertain)

- location: `src/zopfli/squeeze.c:429–444` · severity: info · affects: u-blocksplitter-deflate-squeeze
- LZ77OptimalRun: heap ownership crosses the function boundary (calls free)
- triage (uncertain, low): LZ77OptimalRun's slice shows buffer use but not enough of the allocation/free pairing to decide whether ownership stays local; needs the surrounding ZopfliLZ77Optimal context.

### f-dbc5ddcc657bdbf2 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/squeeze.c:446–526` · severity: info · affects: u-blocksplitter-deflate-squeeze
- ZopfliLZ77Optimal: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): ZopfliLZ77Optimal manages multiple resources (costs, path, current store, hash, stats) with interleaved init/clean lifecycles across iterations; the alloc/free pairing is correct but complex enough that a migration must restructure it around owned types to stay provably leak/double-free safe.

### f-fba219f78fa4a0bf — alloc `alloc-ownership` (dismissed — pending human review)

- location: `src/zopfli/squeeze.c:528–560` · severity: info · affects: u-blocksplitter-deflate-squeeze
- ZopfliLZ77OptimalFixed: heap ownership crosses the function boundary (calls free)
- triage (dismiss, medium): ZopfliLZ77OptimalFixed allocates path/costs locally and frees them before returning; contained pair mapping to owned Vecs.

### f-83530aadc06a51c9 — alloc `alloc-ownership` (dismissed — pending human review)

- location: `src/zopfli/tree.c:30–69` · severity: info · affects: u-tree
- ZopfliLengthsToSymbols: heap ownership crosses the function boundary (calls free)
- triage (dismiss, medium): ZopfliLengthsToSymbols allocates bl_count/next_code and frees both before returning on all paths; contained local pair, direct Vec translation.

### f-51f3d204239b2d45 — macros `macro-statement-body` (confirmed)

- location: `src/zopfli/util.h:146–154` · severity: high · affects: u-cache, u-hash, u-lz77, u-tree, u-blocksplitter-deflate-squeeze, u-gzip_container, u-util, u-zlib_container, u-zopfli_lib, u-zopfli_bin
- function-like macro ZOPFLI_APPEND_DATA with statement body and embedded allocation
- triage (confirm, high): ZOPFLI_APPEND_DATA (C branch) is a statement-body macro embedding malloc/realloc with amortized doubling keyed to a power-of-two size invariant held only by convention (util.h:146-154). Every expansion site inherits hidden allocation and the caller-owned T** grows under the callee's feet; a faithful Rust migration must replace all expansion sites with Vec growth semantics and re-establish the capacity invariant in types.

### f-bc7c30160acfe6c4 — macros `macro-statement-body` (confirmed)

- location: `src/zopfli/util.h:135–144` · severity: high · affects: u-cache, u-hash, u-lz77, u-tree, u-blocksplitter-deflate-squeeze, u-gzip_container, u-util, u-zlib_container, u-zopfli_lib, u-zopfli_bin
- function-like macro ZOPFLI_APPEND_DATA with statement body and embedded allocation
- triage (confirm, high): The C++ branch of ZOPFLI_APPEND_DATA (util.h:135-144) additionally routes the write through a void** reinterpret cast that the C comment itself flags as a strict-aliasing hazard. Layout/aliasing-sensitive allocation inside a macro is exactly the construct that diverges silently under translation.

### f-003f9d2a0e7d758e — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/zopfli_bin.c:144–219` · severity: info · affects: u-zopfli_bin
- main: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): main frees buffers produced by AddStrings/CompressFile paths; ownership of heap strings is conveyed by convention across several functions.

### f-95bd929ebe59245f — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/zopfli_bin.c:99–126` · severity: info · affects: u-zopfli_bin
- CompressFile: heap ownership crosses the function boundary (calls free)
- triage (confirm, medium): CompressFile frees the buffer LoadFile allocated and the output buffer the compressor allocated — ownership crosses two boundaries by convention.

### f-d2fbcda1b13908e7 — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/zopfli_bin.c:131–138` · severity: info · affects: u-zopfli_bin
- AddStrings: heap ownership crosses the function boundary (malloc + pointer return)
- triage (confirm, high): AddStrings mallocs and returns the pointer for the caller to free — explicit ownership transfer that must become an owned String in Rust.

### f-ee8caf01f8fe478c — alloc `alloc-ownership` (confirmed)

- location: `src/zopfli/zopfli_bin.c:45–79` · severity: info · affects: u-zopfli_bin
- LoadFile: heap ownership crosses the function boundary (calls free)
- triage (confirm, high): LoadFile mallocs a buffer whose ownership transfers to the caller (out/outsize out-params) — the canonical allocate-here-free-there pattern.

### f-b51a4854969c64bd — oracle `ub-reliance` (confirmed)

- location: `src/zopfli/katajainen.c:168–170` · severity: high · affects: u001-katajainen
- LeafComparator truncates a size_t difference to int: not a total order for weights differing >= 2^31 — qsort UB per C11 7.22.5p4 (found by the M0 oracle; Rust port uses a true total order)

### f-7d96e9c2e35275ce — oracle `impl-contract` (confirmed)

- location: `src/zopfli/katajainen.c:143–163` · severity: high · affects: u001-katajainen
- ExtractBitLengths has a fixed counts[16] buffer: implicit contract maxbits <= 15 — the C baseline SIGBUSes beyond it (found by the M0 oracle differential driver)

## Standing caveats

The c-treesitter-v1 suite cannot detect (type information required — libclang frontend material): pointer arithmetic, type punning/cast chains, aliasing assumptions, indirect-call target resolution. Reliance on undefined/implementation-defined behavior is oracle/annotation territory (see annotations).
