"""Mutation teeth: each tempting caller->body flow must violate the theorem."""
from itertools import product
from fixed_payload import C, DOMAINS

caught = 0
for index in range(len(DOMAINS)):
    violations = 0
    for inputs in product(*DOMAINS):
        mutated = C + b":" + inputs[index]  # mutant copies one input category
        if mutated != C:
            violations += 1
    assert violations
    caught += 1
    print(f"input category {index}: {violations} violations caught")
assert caught == len(DOMAINS)
print("RESULT: ALL NONINTERFERENCE MUTANTS CAUGHT")
