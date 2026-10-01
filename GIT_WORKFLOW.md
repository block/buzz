Assuming you've already added the `upstream` remote, here are the exact commands.

## 1. Get the latest Buzz changes and update your `main`

```bash
git checkout main
git fetch upstream
git reset --hard upstream/main
git push --force origin main
```

This makes your local `main` and your fork's `main` exactly match `block/buzz:main`.

---

## 2. Switch back to `orbit` and bring those changes in

```bash
git checkout orbit
git merge main
git push origin orbit
```

If there are merge conflicts, Git will pause the merge. Resolve the conflicts, then run:

```bash
git add .
git commit
git push origin orbit
```

---

# Complete workflow (copy & paste)

```bash
git checkout main
git fetch upstream
git reset --hard upstream/main
git push --force origin main

git checkout orbit
git merge main
git push origin orbit
```

---

## Before doing this, make sure:

* ✅ You have **committed** or **stashed** any uncommitted work on `orbit`.
* ✅ You're okay with `main` being reset to exactly match Buzz (which is the workflow you've chosen).

After running these commands:

* `main` will be identical to `block/buzz:main`.
* `orbit` will contain the latest Buzz code plus all of your Orbit changes.
* You can continue developing on `orbit` and push with:

```bash
git add .
git commit -m "Your commit message"
git push origin orbit
```
