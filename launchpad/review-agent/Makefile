# Containment controls for launchpad-26/buzz#120.
#
# `make test` is the single entry point: CI invokes run_controls.py directly, and this
# target exists so a human (or a verification gate) runs the same suite by the same path
# rather than a hand-picked subset.

.PHONY: test check mutations clean

## Run every control. Exits 1 on failure, 2 if a control could not run at all.
test:
	python3 run_controls.py

## Alias, for muscle memory.
check: test

## The mutation harness alone — slowest control, useful in isolation while iterating.
mutations:
	python3 check_mutations.py

clean:
	rm -rf __pycache__
