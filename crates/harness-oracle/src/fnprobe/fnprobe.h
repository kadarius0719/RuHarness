/* RuHarness features probe (docs/FEATURES-PROBE-REDESIGN.md §3.6): included
   into every translation unit of the probed copy with -include. Harness-owned.
   It only declares: it includes nothing, defines no macro and uses no
   __COUNTER__, so the copy preprocesses to the program's own text (§3.3). */
extern unsigned char *volatile __ruharness_seen;
