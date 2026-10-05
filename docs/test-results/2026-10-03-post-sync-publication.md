# Post-sync publication diagnostic — 2026-10-03

## Question

Can buffered commit publication validate all fallible state before the durable
WAL append without materially degrading the sustained write path?

The baseline was commit `a954e9f`; the candidate was commit
`1eeb6213965340e532917bac3beffb4062d41406`. Five alternating pairs used a
canonical 100,000-key load, 100-byte values, a 64 MiB write buffer, and five
seconds of the uniform W5 workload. This is a short diagnostic, not a release
performance claim.

## Result

| Median of five | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Active operations/s | 701.434 | 698.480 | 0.4% lower |
| Overwrite p50 | 4.943 ms | 4.905 ms | 0.8% lower |
| Overwrite p99 | 11.279 ms | 11.925 ms | 5.7% higher |
| Delete-cycle p99 | 17.614 ms | 17.208 ms | 2.3% lower |

Throughput and p50 show no material regression. The small-sample p99 result is
worse and is retained rather than presented as an improvement. The change was
accepted for its correctness property: lock poisoning, point-index disagreement,
and WAL-end overflow are now rejected before append, while in-memory publication
after a successful sync is infallible. Full-duration W5/W6 measurements remain
required before release conclusions.

Raw outputs are retained outside the repository in
`2026-10-03-postsync-infallible-final-ab` under the local benchmark-evidence
directory.
