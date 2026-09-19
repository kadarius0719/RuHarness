//! Safe-Rust core of zopfli's katajainen.c (boundary package-merge,
//! length-limited Huffman code lengths). Fixture for the executor e2e test.

/// A chain node (also used conceptually for leaves in the C version; here
/// leaves are their own type). `tail` is an index into the node arena.
#[derive(Clone, Copy)]
struct Node {
    /// Total weight (symbol count) of this chain.
    weight: usize,
    /// Previous node(s) of this chain, or None.
    tail: Option<usize>,
    /// Leaf symbol index, or number of leaves before this chain.
    count: i32,
}

#[derive(Clone, Copy)]
struct Leaf {
    weight: usize,
    /// Index of the symbol this leaf represents.
    count: i32,
}

/// Arena allocation standing in for the C node pool (`pool->next++`).
fn alloc_node(nodes: &mut Vec<Node>, node: Node) -> usize {
    nodes.push(node);
    nodes.len() - 1
}

/// Performs a Boundary Package-Merge step: puts a new chain in list `index`;
/// the new chain is a leaf or a combination of two chains from the previous
/// list, depending on the weights.
fn boundary_pm(
    lists: &mut [[usize; 2]],
    leaves: &[Leaf],
    numsymbols: i32,
    nodes: &mut Vec<Node>,
    index: usize,
) {
    let lastcount = nodes[lists[index][1]].count; /* Count of last chain of list. */

    if index == 0 && lastcount >= numsymbols {
        return;
    }

    let oldchain = lists[index][1];
    lists[index][0] = oldchain;

    if index == 0 {
        /* New leaf node in list 0. */
        let leaf = leaves[lastcount as usize];
        let newchain = alloc_node(
            nodes,
            Node {
                weight: leaf.weight,
                count: lastcount + 1,
                tail: None,
            },
        );
        lists[index][1] = newchain;
    } else {
        let sum = nodes[lists[index - 1][0]].weight + nodes[lists[index - 1][1]].weight;
        if lastcount < numsymbols && sum > leaves[lastcount as usize].weight {
            /* New leaf inserted in list, so count is incremented. */
            let tail = nodes[oldchain].tail;
            let leaf = leaves[lastcount as usize];
            let newchain = alloc_node(
                nodes,
                Node {
                    weight: leaf.weight,
                    count: lastcount + 1,
                    tail,
                },
            );
            lists[index][1] = newchain;
        } else {
            let newchain = alloc_node(
                nodes,
                Node {
                    weight: sum,
                    count: lastcount,
                    tail: Some(lists[index - 1][1]),
                },
            );
            lists[index][1] = newchain;
            /* Two lookahead chains of previous list used up, create new ones. */
            boundary_pm(lists, leaves, numsymbols, nodes, index - 1);
            boundary_pm(lists, leaves, numsymbols, nodes, index - 1);
        }
    }
}

fn boundary_pm_final(
    lists: &mut [[usize; 2]],
    leaves: &[Leaf],
    numsymbols: i32,
    nodes: &mut Vec<Node>,
    index: usize,
) {
    let lastcount = nodes[lists[index][1]].count; /* Count of last chain of list. */
    let sum = nodes[lists[index - 1][0]].weight + nodes[lists[index - 1][1]].weight;

    if lastcount < numsymbols && sum > leaves[lastcount as usize].weight {
        /* The C original leaves this node's weight uninitialized (it is never
        read); 0 here. */
        let oldchain = nodes[lists[index][1]].tail;
        let newchain = alloc_node(
            nodes,
            Node {
                weight: 0,
                count: lastcount + 1,
                tail: oldchain,
            },
        );
        lists[index][1] = newchain;
    } else {
        let prev = lists[index - 1][1];
        nodes[lists[index][1]].tail = Some(prev);
    }
}

/// Initializes each list with the two lightest leaves as lookahead chains.
fn init_lists(nodes: &mut Vec<Node>, leaves: &[Leaf], lists: &mut [[usize; 2]]) {
    let node0 = alloc_node(
        nodes,
        Node {
            weight: leaves[0].weight,
            count: 1,
            tail: None,
        },
    );
    let node1 = alloc_node(
        nodes,
        Node {
            weight: leaves[1].weight,
            count: 2,
            tail: None,
        },
    );
    for list in lists.iter_mut() {
        *list = [node0, node1];
    }
}

/// Converts the result of boundary package-merge to bitlengths. The last
/// chain of the last list contains the number of active leaves per list.
fn extract_bit_lengths(chain: usize, leaves: &[Leaf], bitlengths: &mut [u32], nodes: &[Node]) {
    let mut counts = [0i32; 16];
    let mut end = 16usize;
    let mut ptr = 15usize;
    let mut value = 1u32;

    let mut node = Some(chain);
    while let Some(cur) = node {
        end -= 1;
        counts[end] = nodes[cur].count;
        node = nodes[cur].tail;
    }

    let mut val = counts[15];
    while ptr >= end {
        while val > counts[ptr - 1] {
            bitlengths[leaves[(val - 1) as usize].count as usize] = value;
            val -= 1;
        }
        ptr -= 1;
        value += 1;
    }
}

/// Safe-Rust core. See `katajainen.h` for the contract; `frequencies` and
/// `bitlengths` have the same length.
pub fn length_limited_code_lengths(
    frequencies: &[usize],
    maxbits: i32,
    bitlengths: &mut [u32],
) -> i32 {
    /* Initialize all bitlengths at 0. */
    for b in bitlengths.iter_mut() {
        *b = 0;
    }

    /* Count used symbols and place them in the leaves. */
    let mut leaves: Vec<Leaf> = Vec::new();
    for (i, &f) in frequencies.iter().enumerate() {
        if f != 0 {
            leaves.push(Leaf {
                weight: f,
                count: i as i32,
            });
        }
    }
    let numsymbols = leaves.len() as i32;

    /* Check special cases and error conditions. */
    if (1i32 << maxbits) < numsymbols {
        return 1; /* Error, too few maxbits to represent symbols. */
    }
    if numsymbols == 0 {
        return 0; /* No symbols at all. OK. */
    }
    if numsymbols == 1 {
        bitlengths[leaves[0].count as usize] = 1;
        return 0; /* Only one symbol, give it bitlength 1, not 0. OK. */
    }
    if numsymbols == 2 {
        bitlengths[leaves[0].count as usize] += 1;
        bitlengths[leaves[1].count as usize] += 1;
        return 0;
    }

    /* Sort the leaves from lightest to heaviest, count packed into the low
    9 bits for stable sorting, exactly as the C version does. */
    for leaf in leaves.iter() {
        if leaf.weight >= 1usize << (usize::BITS - 9) {
            return 1; /* Error, we need 9 bits for the count. */
        }
    }
    for leaf in leaves.iter_mut() {
        leaf.weight = (leaf.weight << 9) | (leaf.count as usize);
    }
    /* The C comparator truncates the size_t difference to int, which is not
    a total order once weights differ by >= 2^31 — undefined behavior for
    qsort (C11 7.22.5p4). Rust uses the true total order instead: identical
    results everywhere the C comparator is consistent (all weights < 2^22
    pre-shift), well-defined behavior beyond that. See migration/DECISIONS.md
    hazard #1. Keys are unique (count packed in the low 9 bits), so unstable
    sort vs qsort makes no difference. */
    leaves.sort_unstable_by_key(|leaf| leaf.weight);
    for leaf in leaves.iter_mut() {
        leaf.weight >>= 9;
    }

    let maxbits = maxbits.min(numsymbols - 1) as usize;

    /* Node arena, sized like the C pool. */
    let mut nodes: Vec<Node> = Vec::with_capacity(maxbits * 2 * numsymbols as usize);
    let mut lists: Vec<[usize; 2]> = vec![[0, 0]; maxbits];
    init_lists(&mut nodes, &leaves, &mut lists);

    /* In the last list, 2 * numsymbols - 2 active chains need to be created;
    two exist from initialization, each BoundaryPM run creates one. */
    let num_boundary_pm_runs = 2 * numsymbols - 4;
    for _ in 0..num_boundary_pm_runs - 1 {
        boundary_pm(&mut lists, &leaves, numsymbols, &mut nodes, maxbits - 1);
    }
    boundary_pm_final(&mut lists, &leaves, numsymbols, &mut nodes, maxbits - 1);

    extract_bit_lengths(lists[maxbits - 1][1], &leaves, bitlengths, &nodes);
    0 /* OK. */
}
