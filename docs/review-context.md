# Review context (`rtok review`)

`rtok review` builds a token-efficient prompt for a reviewer model: the diff plus signature
digests of the code it touches, then optionally runs the review. Full docs (how it works,
why, evaluation, enable/disable, tests, limitations): [src/review/README.md](../src/review/README.md).
For the model-free reading list, see [`rtok graph review`](review.md).
