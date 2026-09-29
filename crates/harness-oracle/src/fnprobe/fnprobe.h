/* RuHarness features probe (docs/FEATURES-DESIGN.md §5.4): included into
   every translation unit of the probed copy with -include. Harness-owned. */
extern unsigned char __ruharness_seen[];
void __ruharness_probe(unsigned id);
