#!/bin/bash
# usage: run_batch.sh <batchmap-file> <batch> <stage-cmd> <model> <round>
T=$(cd "$(dirname "$0")" && pwd)   # tools
S="${HANDOFF_ROOT:?set HANDOFF_ROOT}"   # batch dirs, maps, indexes
grep "^$2 " "$S/$1" > "$S/bm-$2.txt"
python3 "$T/import_batches.py" "$S/bm-$2.txt" "$3" "$5" "$4"
ONLY=$(python3 -c "
import json;idx=json.load(open('$S/handoff-index/$2.json'));print(','.join(sorted({p.split('/cases/')[1].split('/migration/')[0].split('/')[-1] for p in idx.values()})))")
python3 "$T/stage.py" "$3" "$4" --jobs 5 --only "$ONLY" | tail -12
cp "$S/awaited-$3.txt" "$S/awaited-$3-$2.txt"
