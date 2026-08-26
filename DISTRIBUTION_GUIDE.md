# Buzz Desktop & Server Distribution Guide

This guide explains how to package and distribute the Buzz Desktop App (`.exe`) and the Buzz Relay Server (using Docker) to another person **without exposing your desktop application source code**.

---

## Architecture Overview

```
[ Your Machine (Developer) ]
  ├─ 1. Build Desktop App ───► Buzz_x64-setup.exe (Compiled binary, no source code)
  └─ 2. Prepare Server ──────► buzz-server.zip (docker-compose.yml + .env + start.bat)
                                         │
                                         ▼
[ Recipient Machine ]
  ├─ Run `start.bat` (Starts Server via Docker on port 3000)
  └─ Run `Buzz_x64-setup.exe` (Connects automatically to local server)
```

---

## Part 1: Build the Desktop `.exe`

The desktop application is built using Tauri 2. When built in release mode, all React and TypeScript code is compiled directly into a native Windows binary.

### 1. Prepare Sidecar Binaries (Required)
Tauri validates the presence of sidecar binaries at build time. On Windows, run the following commands in PowerShell from the project root:

```powershell
# 1. Build release binaries for CLI & sidecars
cargo build --release -p buzz -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr

# 2. Create the binaries directory in desktop/src-tauri
New-Item -ItemType Directory -Force -Path "desktop\src-tauri\binaries"

# 3. Copy binaries with the Windows target triple suffix (x86_64-pc-windows-msvc)
$TARGET = "x86_64-pc-windows-msvc"
Copy-Item "target\release\buzz.exe" "desktop\src-tauri\binaries\buzz-$TARGET.exe"
Copy-Item "target\release\buzz-acp.exe" "desktop\src-tauri\binaries\buzz-acp-$TARGET.exe"
Copy-Item "target\release\buzz-agent.exe" "desktop\src-tauri\binaries\buzz-agent-$TARGET.exe"
Copy-Item "target\release\buzz-dev-mcp.exe" "desktop\src-tauri\binaries\buzz-dev-mcp-$TARGET.exe"
Copy-Item "target\release\git-credential-nostr.exe" "desktop\src-tauri\binaries\git-credential-nostr-$TARGET.exe"

# Create a placeholder stub for the unused kubernetes backend sidecar
New-Item -ItemType File -Force -Path "desktop\src-tauri\binaries\buzz-backend-kubernetes-$TARGET.exe"
```

### 2. Run the Build Command
Navigate to the `desktop` folder and run the Tauri release build:

```powershell
cd desktop
pnpm install
pnpm tauri build
```

### 3. Locate the Output Files
Send one of the following generated files to the recipient:
* **NSIS Installer**: `desktop\src-tauri\target\release\bundle\nsis\Buzz_0.5.4_x64-setup.exe`
* **Standalone Executable**: `desktop\src-tauri\target\release\buzz.exe`

---

## Part 2: Package the Server for Docker

Create a new directory called `buzz-server/` on your computer. Inside this folder, create the following four files. This folder will be zipped and sent to the recipient.

### 1. `docker-compose.yml`
```yaml
name: buzz-server

services:
  relay:
    image: ${BUZZ_IMAGE:-ghcr.io/block/buzz:main}
    env_file:
      - .env
    environment:
      BUZZ_BIND_ADDR: 0.0.0.0:3000
      BUZZ_HEALTH_PORT: "8080"
      BUZZ_METRICS_PORT: "9102"
      DATABASE_URL: postgres://${POSTGRES_USER:-buzz}:${POSTGRES_PASSWORD:-buzz_secret}@postgres:5432/${POSTGRES_DB:-buzz}
      REDIS_URL: redis://:${REDIS_PASSWORD:-redis_secret}@redis:6379
      BUZZ_S3_ENDPOINT: http://minio:9000
      BUZZ_S3_ADDRESSING_STYLE: path
      BUZZ_S3_ACCESS_KEY: ${BUZZ_S3_ACCESS_KEY:-buzz_dev}
      BUZZ_S3_SECRET_KEY: ${BUZZ_S3_SECRET_KEY:-buzz_dev_secret}
      BUZZ_S3_BUCKET: ${BUZZ_S3_BUCKET:-buzz-media}
      BUZZ_GIT_REPO_PATH: /data/git
      BUZZ_AUTO_MIGRATE: "true"
    ports:
      - "3000:3000"
    volumes:
      - buzz-git-data:/data/git
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
      minio:
        condition: service_healthy
      minio-init:
        condition: service_completed_successfully
    restart: unless-stopped
    networks:
      - buzz-net

  postgres:
    image: postgres:17-alpine
    environment:
      POSTGRES_DB: ${POSTGRES_DB:-buzz}
      POSTGRES_USER: ${POSTGRES_USER:-buzz}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-buzz_secret}
      PGDATA: /var/lib/postgresql/data/pgdata
    volumes:
      - buzz-postgres-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U $${POSTGRES_USER} -d $${POSTGRES_DB}"]
      interval: 5s
      timeout: 5s
      retries: 12
      start_period: 10s
    restart: unless-stopped
    networks:
      - buzz-net

  redis:
    image: redis:7-alpine
    command: ["redis-server", "--appendonly", "yes", "--requirepass", "${REDIS_PASSWORD:-redis_secret}"]
    environment:
      REDIS_PASSWORD: ${REDIS_PASSWORD:-redis_secret}
    volumes:
      - buzz-redis-data:/data
    healthcheck:
      test: ["CMD-SHELL", "redis-cli -a \"$${REDIS_PASSWORD}\" ping | grep -q PONG"]
      interval: 5s
      timeout: 3s
      retries: 12
      start_period: 5s
    restart: unless-stopped
    networks:
      - buzz-net

  minio:
    image: minio/minio:RELEASE.2025-09-07T16-13-09Z
    command: server /data --console-address ":9001"
    environment:
      MINIO_ROOT_USER: ${BUZZ_S3_ACCESS_KEY:-buzz_dev}
      MINIO_ROOT_PASSWORD: ${BUZZ_S3_SECRET_KEY:-buzz_dev_secret}
    volumes:
      - buzz-minio-data:/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://127.0.0.1:9000/minio/health/live"]
      interval: 5s
      timeout: 5s
      retries: 12
      start_period: 10s
    restart: unless-stopped
    networks:
      - buzz-net

  minio-init:
    image: minio/mc:RELEASE.2025-08-13T08-35-41Z
    depends_on:
      minio:
        condition: service_healthy
    environment:
      BUZZ_S3_ACCESS_KEY: ${BUZZ_S3_ACCESS_KEY:-buzz_dev}
      BUZZ_S3_SECRET_KEY: ${BUZZ_S3_SECRET_KEY:-buzz_dev_secret}
      BUZZ_S3_BUCKET: ${BUZZ_S3_BUCKET:-buzz-media}
    entrypoint: >
      /bin/sh -euc '
        mc alias set local http://minio:9000 "$${BUZZ_S3_ACCESS_KEY}" "$${BUZZ_S3_SECRET_KEY}"
        mc mb --ignore-existing "local/$${BUZZ_S3_BUCKET}"
        mc anonymous set none "local/$${BUZZ_S3_BUCKET}"
      '
    restart: "no"
    networks:
      - buzz-net

volumes:
  buzz-postgres-data:
  buzz-redis-data:
  buzz-minio-data:
  buzz-git-data:

networks:
  buzz-net:
    driver: bridge
```

### 2. `.env`
```ini
BUZZ_IMAGE=ghcr.io/block/buzz:main
BUZZ_DOMAIN=localhost:3000
RELAY_URL=ws://localhost:3000
BUZZ_MEDIA_BASE_URL=http://localhost:3000/media
BUZZ_MEDIA_SERVER_DOMAIN=localhost:3000
BUZZ_CORS_ORIGINS=http://localhost:3000

BUZZ_REQUIRE_AUTH_TOKEN=false
BUZZ_REQUIRE_RELAY_MEMBERSHIP=false
BUZZ_AUTO_MIGRATE=true

POSTGRES_DB=buzz
POSTGRES_USER=buzz
POSTGRES_PASSWORD=buzz_secret
REDIS_PASSWORD=redis_secret
BUZZ_S3_ACCESS_KEY=buzz_dev
BUZZ_S3_SECRET_KEY=buzz_dev_secret
BUZZ_S3_BUCKET=buzz-media
```

### 3. `start.bat`
```bat
@echo off
echo ===================================================
echo Starting Buzz Relay Server...
echo ===================================================
docker compose up -d
echo.
echo Server is running on http://localhost:3000
echo You can now launch the Buzz Desktop App!
pause
```

### 4. `stop.bat`
```bat
@echo off
echo Stopping Buzz Relay Server...
docker compose down
echo Server stopped.
pause
```

---

## Part 3: Instructions for the Recipient

Send the recipient the **`.exe` file** and the **zipped `buzz-server` folder**. They should execute the following steps to run them:

1. **Install Docker**: Download and start [Docker Desktop for Windows](https://www.docker.com/products/docker-desktop/).
2. **Start the Server**: Extract `buzz-server.zip`, open the folder, and double-click **`start.bat`**. This will download the images (the first time) and launch the server in the background.
3. **Launch the Client**: Install/run **`Buzz_0.5.4_x64-setup.exe`** (or the standalone `buzz.exe`). The app will automatically connect to your local backend on `ws://localhost:3000`.
4. **Shutdown**: When done, double-click **`stop.bat`** to clean up the running containers.
