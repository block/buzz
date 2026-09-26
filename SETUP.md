# Buzz Development & Setup Guide

This guide provides a comprehensive walkthrough for setting up, running, and building the Buzz ecosystem locally on a Windows PC.

---

## 🏗️ Architecture Overview

The Buzz project spans multiple components structured within a monorepo workspace:

*   **`crates/`**: The core backend Rust services.
    *   `buzz-relay`: WebSocket relay server (NIP-29 protocol).
    *   `buzz-db`: Postgres access layer.
    *   `buzz-cli`: Command Line Interface for agents/users.
*   **`desktop/`**: Tauri 2 + React 19 desktop application wrapper.
*   **`web/`**: Browser-based repository viewer/client.
*   **`migrations/`**: SQL database migration files.

---

## 📋 Prerequisites

Ensure the following tools are installed and configured on your Windows system:

| Tool | Required Version | Status / Action |
| :--- | :--- | :--- |
| **Docker Desktop** | Latest | *User Action: Pre-installed & Running* |
| **C++ Build Tools** | MSVC v143+ | *User Action: Pre-installed via VS Build Tools* |
| **CMake** | Latest | Install via `winget install -e --id Kitware.CMake` (Ensure it is in your system `PATH`) |
| **Rust Toolchain** | 1.88+ | Install via [rustup.rs](https://rustup.rs/) |
| **Node.js** | 24+ | Download from [nodejs.org](https://nodejs.org/) |
| **pnpm** | 10+ | Install globally: `npm install -g pnpm` |
| **Just** | Latest | Install task runner: `cargo install just` |

---

## 🏃 Quick Start: How to Run and Test the Desktop App

To test the desktop app, follow these steps in order.

### Step 1: Open Git Bash or a Bash Terminal
Because the project's task runner (`just`) relies on Bash interpreter scripts, you **must run these commands from Git Bash** (or Windows PowerShell with Git's `usr/bin` folder added to your environment `PATH` variable).

1. Open **Git Bash**.
2. Change directory to the repository root:
   ```bash
   cd "c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit"
   ```

### Step 2: Start Docker Desktop & Configure PATH
Before running the setup commands, you **must** ensure Docker is running and available in your terminal's environment (otherwise `just bootstrap` will fail with "Docker is required but not installed").

1. **Launch Docker Desktop**: Open the Windows Start menu, search for **Docker Desktop**, and open it. Wait until the Docker engine shows as "Running" (green icon in the bottom left).
2. **Enable CLI Integration (Crucial for Git Bash)**:
   *   In Docker Desktop, click the **Gear icon (Settings)** in the top right.
   *   Go to **Advanced** (or **General**, depending on version).
   *   Ensure the setting to **Add Docker CLI tools to PATH** (or "User/System PATH") is **checked/enabled**.
   *   *Note: If you just enabled this, you **MUST restart Git Bash** completely for it to detect the `docker` command.*

### Step 3: One-Time Bootstrap & Setup (Workspace Root)
Once Docker is running and your terminal can recognize the `docker` command, prepare the environment.

1. **Bootstrap the environment configurations**:
   *   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
   *   **Command**:
       ```bash
       just bootstrap
       ```
2. **Provision local databases, run migrations, and install JS dependencies**:
   *   *Note: This step downloads all required Docker images (Postgres, Redis, etc.) and starts them.*
   *   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
   *   **Command**:
       ```bash
       just setup
       ```

### Step 3: Run the Application (Choose one method below)

#### Method A: Full App & Local Relay Server (Recommended for Backend + Frontend testing)
This runs the local WebSocket relay server (connected to Postgres & Redis in Docker) and automatically launches the compiled Tauri Desktop App wrapper.
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just dev
    ```
*   **What to expect**: A window containing the Tauri desktop client should pop up. The command terminal will output active relay server logs.

#### Method B: Standalone Desktop App (Frontend Only, connecting to Remote/Public Relay)
If you want to run the native application window without spinning up local databases or the backend relay:
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just desktop-standalone
    ```

#### Method C: Web Dev Server (React Frontend in the Browser)
If you just want to test and develop the React layout inside your Chrome/Edge browser instead of compiling the native Tauri wrapper:
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit/desktop`
*   **Command**:
    ```bash
    pnpm dev
    ```
*   **What to expect**: Open [http://localhost:5173](http://localhost:5173) in your browser.

#### Method D: Manual Execution (Without `just` runner)
If you prefer not to use the `just` task runner and want to navigate and start the services manually:

##### 1. Start the Relay Server Manually
1. Open Git Bash (or your preferred shell) and start the Docker services:
   ```bash
   docker compose up -d
   ```
2. Apply database migrations:
   ```bash
   cargo run -p buzz-admin -- migrate
   ```
3. Seed the local community data:
   ```bash
   ./scripts/seed-local-community.sh
   ```
4. Start the backend relay server:
   ```bash
   cd crates/buzz-relay
   cargo run
   ```

##### 2. Start the Desktop Client Manually
1. In a separate terminal, ensure sidecar placeholders exist (required by Tauri at compile time):
   * **On Windows (Git Bash)**:
     ```bash
     mkdir -p desktop/src-tauri/binaries
     TARGET=$(rustc -vV | sed -n 's|host: ||p')
     for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
         touch "desktop/src-tauri/binaries/${bin}-${TARGET}.exe"
     done
     ```
   * **On macOS/Linux**:
     ```bash
     mkdir -p desktop/src-tauri/binaries
     TARGET=$(rustc -vV | sed -n 's|host: ||p')
     for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
         touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
     done
     ```
2. Navigate to the `desktop` folder, load the environment configuration, and run the Tauri dev command:
   ```bash
   cd desktop
   pnpm install
   source ../scripts/instance-env.sh
   pnpm exec tauri dev --config "$BUZZ_TAURI_CONFIG"
   ```

---

## ⚙️ First-Time Setup Details

Detailed setup options run from the root directory `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`:

### 1. Set Up Git Commit Hooks (Optional)
Install the pre-commit hooks that automatically format code and check for issues before commits:
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just hooks
    ```

---

## 🚀 Detailed Workspace Commands & Locations

Below is a cheat sheet indicating exactly **where** to run each command:

| Goal / Action | Command to Run | Target Directory |
| :--- | :--- | :--- |
| **Full Stack Dev (Relay + Tauri App)** | `just dev` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |
| **Standalone Tauri App** | `just desktop-standalone` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |
| **Launch Local Web Client** | `pnpm dev` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit/desktop` |
| **Start Only Local Relay** | `just relay` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |
| **Show Docker Container Status** | `just ps` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |
| **Stop All Docker Containers** | `just down` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |
| **Factory Reset Database/State** | `just reset` | `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit` |

---

## 🛠️ Building the Desktop Application

To compile the application into a distribution-ready installer (`.exe`/`.msi`):

### Option 1: Using the Just task runner
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just desktop-release-build target="x86_64-pc-windows-msvc"
    ```

### Option 2: Manually building via Tauri CLI
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit/desktop`
*   **Commands**:
    ```bash
    pnpm install
    pnpm tauri build
    ```
The compiled installer will be saved to:
`c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit/desktop/src-tauri/target/release/bundle/`

---

## 🧹 Maintenance & Troubleshooting

### Tail Docker Logs
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just logs
    ```

### Run CI Checks Locally
To verify everything compiles and formats correctly before pushing code:
*   **Directory**: `c:/Users/Yash Avsarmal/Downloads/orbit-main/orbit`
*   **Command**:
    ```bash
    just ci
    ```
*(If formatting or lints fail, run `just fix-all` to auto-format files workspace-wide)*

