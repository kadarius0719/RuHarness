// Length-limited Huffman code lengths via the boundary package-merge
// algorithm, translated from zopfli's katajainen.c. Based on the paper
// "A Fast and Space-Economical Algorithm for Length-Limited Coding" by
// Jyrki Katajainen, Alistair Moffat and Andrew Turpin.
//
// The C node pool (pointers into one allocated block) is modelled as a vector
// of nodes addressed by index; a chain tail is an optional index.

/// Node forming chains.
#[derive(Clone, Copy)]
struct Node {
    /// Total weight (symbol count) of this chain.
    weight: usize,
    /// Previous node of this chain (index into the pool), or None.
    tail: Option<usize>,
    /// Number of leaves before this chain.
    count: i32,
}

/// One leaf per used symbol.
#[derive(Clone, Copy)]
struct Leaf {
    /// Symbol frequency.
    weight: usize,
    /// Index of the symbol this leaf represents.
    count: i32,
}

/// Performs a boundary package-merge step: puts a new chain in the list with
/// index start_index. The new chain is, depending on the weights, a leaf or a
/// combination of two chains from the previous list. The C code calls itself
/// twice on the previous list as its final action; the explicit pending stack
/// performs those calls in the identical depth-first order.
fn boundary_pm(
    lists: &mut [[usize; 2]],
    leaves: &[Leaf],
    numsymbols: i32,
    pool: &mut Vec<Node>,
    pending: &mut Vec<usize>,
    start_index: usize,
) {
    pending.clear();
    pending.push(start_index);

    while let Some(index) = pending.pop() {
        let oldchain = lists[index][1];
        // Count of last chain of list.
        let lastcount = pool[oldchain].count;

        if index == 0 && lastcount >= numsymbols {
            continue;
        }

        let newchain = pool.len();
        lists[index][0] = oldchain;
        lists[index][1] = newchain;

        if index == 0 {
            // New leaf node in list 0.
            pool.push(Node {
                weight: leaves[lastcount as usize].weight,
                tail: None,
                count: lastcount.wrapping_add(1),
            });
        } else {
            let prev0 = lists[index - 1][0];
            let prev1 = lists[index - 1][1];
            let sum = pool[prev0].weight.wrapping_add(pool[prev1].weight);
            if lastcount < numsymbols && sum > leaves[lastcount as usize].weight {
                // New leaf inserted in list, so count is incremented.
                let tail = pool[oldchain].tail;
                pool.push(Node {
                    weight: leaves[lastcount as usize].weight,
                    tail,
                    count: lastcount.wrapping_add(1),
                });
            } else {
                pool.push(Node {
                    weight: sum,
                    tail: Some(prev1),
                    count: lastcount,
                });
                // Two lookahead chains of previous list used up, create new ones.
                pending.push(index - 1);
                pending.push(index - 1);
            }
        }
    }
}

/// Final boundary package-merge step for the last list: only the count and
/// tail of the resulting last chain are needed afterwards.
fn boundary_pm_final(
    lists: &mut [[usize; 2]],
    leaves: &[Leaf],
    numsymbols: i32,
    pool: &mut Vec<Node>,
    index: usize,
) {
    let last = lists[index][1];
    // Count of last chain of list.
    let lastcount = pool[last].count;

    let prev0 = lists[index - 1][0];
    let prev1 = lists[index - 1][1];
    let sum = pool[prev0].weight.wrapping_add(pool[prev1].weight);

    if lastcount < numsymbols && sum > leaves[lastcount as usize].weight {
        let oldchain = pool[last].tail;
        let newchain = pool.len();
        // The C code leaves the weight of this node unset; it is never read.
        pool.push(Node {
            weight: 0,
            tail: oldchain,
            count: lastcount.wrapping_add(1),
        });
        lists[index][1] = newchain;
    } else {
        pool[last].tail = Some(prev1);
    }
}

/// Converts the result of boundary package-merge to the bit lengths. The last
/// chain of the last list holds the number of active leaves in each list. The
/// C code copies the chain counts into the top of a zero-filled 16-entry array
/// and compares against the entry below; walking the chain directly and using
/// 0 after the chain end yields the identical sequence of writes for every
/// chain that fits that array.
fn extract_bit_lengths(chain: usize, pool: &[Node], leaves: &[Leaf], bitlengths: &mut [u32]) {
    let mut value: u32 = 1;
    let mut val: i32 = pool[chain].count;
    let mut node = Some(chain);

    while let Some(idx) = node {
        let next = pool[idx].tail;
        let threshold: i32 = match next {
            Some(t) => pool[t].count,
            None => 0,
        };
        while val > threshold {
            let leaf = leaves[(val - 1) as usize];
            bitlengths[leaf.count as usize] = value;
            val -= 1;
        }
        value = value.wrapping_add(1);
        node = next;
    }
}

/// Outputs minimum-redundancy length-limited code bit lengths for symbols with
/// the given frequencies, limited by maxbits. The symbol count n of the C
/// signature is the slice length. Returns 0 for OK, non-0 for error.
pub fn length_limited_code_lengths(
    frequencies: &[usize],
    maxbits: i32,
    bitlengths: &mut [u32],
) -> i32 {
    let n = frequencies.len().min(bitlengths.len());
    let mut maxbits = maxbits;

    // Initialize all bitlengths at 0.
    for length in bitlengths[..n].iter_mut() {
        *length = 0;
    }

    // Count used symbols and place them in the leaves.
    let mut leaves: Vec<Leaf> = Vec::with_capacity(n);
    for (i, &frequency) in frequencies[..n].iter().enumerate() {
        if frequency != 0 {
            leaves.push(Leaf {
                weight: frequency,
                // Index of symbol this leaf represents.
                count: i as i32,
            });
        }
    }
    // Amount of symbols with frequency > 0.
    let numsymbols = leaves.len() as i32;

    // Check special cases and error conditions.
    if 1i32.wrapping_shl(maxbits as u32) < numsymbols {
        return 1; // Error, too few maxbits to represent symbols.
    }
    if numsymbols == 0 {
        return 0; // No symbols at all. OK.
    }
    if numsymbols == 1 {
        bitlengths[leaves[0].count as usize] = 1;
        return 0; // Only one symbol, give it bitlength 1, not 0. OK.
    }
    if numsymbols == 2 {
        let first = leaves[0].count as usize;
        let second = leaves[1].count as usize;
        bitlengths[first] = bitlengths[first].wrapping_add(1);
        bitlengths[second] = bitlengths[second].wrapping_add(1);
        return 0;
    }

    // Sort the leaves from lightest to heaviest. The count is merged into the
    // weight so the ordering is total. The C comparator returns the key
    // difference narrowed to int, which orders correctly whenever it is well
    // defined; that ordering (ascending by merged key) is implemented here.
    let limit: usize = 1usize << (usize::BITS - 9);
    for leaf in leaves.iter_mut() {
        if leaf.weight >= limit {
            return 1; // Error, we need 9 bits for the count.
        }
        leaf.weight = (leaf.weight << 9) | (leaf.count as usize);
    }
    leaves.sort_by_key(|leaf| leaf.weight);
    for leaf in leaves.iter_mut() {
        leaf.weight >>= 9;
    }

    if numsymbols - 1 < maxbits {
        maxbits = numsymbols - 1;
    }
    // Shift counts outside the width of int are not defined in C; such a
    // maxbits can arrive here below 2, where no list structure exists.
    if maxbits < 2 {
        return 1;
    }

    let nlists = maxbits as usize;
    let symbols = numsymbols as usize;

    // Initialize node memory pool.
    let capacity = nlists.saturating_mul(2).saturating_mul(symbols);
    let mut pool: Vec<Node> = Vec::with_capacity(capacity.min(1 << 16));

    // Initialize each list with as lookahead chains the two leaves with lowest
    // weights.
    pool.push(Node {
        weight: leaves[0].weight,
        tail: None,
        count: 1,
    });
    pool.push(Node {
        weight: leaves[1].weight,
        tail: None,
        count: 2,
    });
    let mut lists: Vec<[usize; 2]> = vec![[0usize, 1usize]; nlists];

    // In the last list, 2 * numsymbols - 2 active chains need to be created. Two
    // are already created in the initialization. Each boundary_pm run creates one.
    let num_boundary_pm_runs = numsymbols.wrapping_mul(2).wrapping_sub(4);
    let mut pending: Vec<usize> = Vec::new();
    let mut i: i32 = 0;
    while i < num_boundary_pm_runs.wrapping_sub(1) {
        boundary_pm(&mut lists, &leaves, numsymbols, &mut pool, &mut pending, nlists - 1);
        i += 1;
    }
    boundary_pm_final(&mut lists, &leaves, numsymbols, &mut pool, nlists - 1);

    extract_bit_lengths(lists[nlists - 1][1], &pool, &leaves, bitlengths);

    0 // OK.
}
