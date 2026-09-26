# Dev-Suite Project Plan

A comprehensive development toolkit that provides both a React UI overlay and a terminal interface for debugging, testing, monitoring, and DevOps integration.

## Project Overview

**Goal**: Create a multi-modal Node package for development workflows:

### Core Features (v1.0)
- Error log viewer
- Color palette explorer
- Component catalog
- Data flow visualization (frontend/backend)
- Data watch/debug logs
- Git log viewer
- HMR enhancement (state preservation, status monitoring)

### Extended Features (v2.0)
- Terminal UI (TUI) for CLI-based development
- Full DevOps suite integration
- Test automation & assessment
- CI/CD platform connectors

---

## Roadmap

| Version | Scope | Phases |
|---------|-------|--------|
| v1.0 | React Dev UI | Phases 1-6 |
| v1.5 | TUI Addition | Phase 7 |
| v2.0 | DevOps Suite | Phase 8 |

---

## Phase 1: Project Setup & Core Infrastructure

### 1.1 Initialize Package
- [ ] `npm init` with package name `dev-suite` or `@your-scope/dev-suite`
- [ ] Configure `package.json` for dual ESM/CommonJS support
- [ ] Set up TypeScript with strict mode
- [ ] Configure build system (tsup or rollup for bundling)

### 1.2 Project Structure
```
dev-suite/
├── src/
│   ├── index.ts              # Main exports
│   ├── components/
│   │   ├── DevSuite.tsx      # Main wrapper component
│   │   ├── Panel.tsx         # Side/bottom panel container
│   │   ├── ErrorLog/
│   │   ├── ColorPalette/
│   │   ├── ComponentCatalog/
│   │   ├── DataFlow/
│   │   ├── DataWatch/
│   │   ├── GitLog/
│   │   └── HMRMonitor/
│   ├── hooks/
│   │   ├── useErrorCapture.ts
│   │   ├── useGitLog.ts
│   │   ├── useDataWatch.ts
│   │   └── useHMR.ts
│   ├── context/
│   │   └── DevSuiteContext.tsx
│   ├── utils/
│   │   ├── parser.ts
│   │   └── formatters.ts
│   └── types/
│       └── index.ts
├── package.json
├── tsconfig.json
├── tsup.config.ts
└── README.md
```

---

## Phase 2: Core Features

### 2.1 DevSuite Wrapper Component
- Floating toggle button (draggable)
- Collapsible side/bottom panel
- Tab-based navigation between features
- Keyboard shortcuts (e.g., Ctrl+Shift+D to toggle)
- Theme support (light/dark)

### 2.2 Error Log Viewer
- Capture `window.onerror`, `unhandledrejection`, React error boundaries
- Display stack traces with source map support
- Filter by error type, time range
- Clear/export logs
- Link to source files (open in editor)

### 2.3 Color Palette Explorer
- Auto-detect colors from CSS/SCSS variables
- Display color swatches with hex/rgb/hsl values
- Copy color values to clipboard
- Contrast checker (WCAG compliance)
- Manual palette import/export

### 2.4 Component Catalog
- Scan `src/components` directory
- Display component tree/list
- Show props interface (TypeScript)
- Component search/filter
- Optional: Storybook integration

### 2.5 Data Flow Visualization
- Frontend: React state/props flow diagram
- Backend: API request/response tracking
- Network tab (fetch/axios interceptor)
- State timeline (Redux/Context/Zustand support)
- Visual graph of data movement

### 2.6 Data Watch / Debug Logs
- `devSuite.log()` helper function
- Watch specific variables/state
- Real-time value updates
- Filter by tag/category
- Timestamp and grouping

### 2.7 Git Log Viewer
- Display recent commits
- Branch visualization
- File change history
- Diff viewer (optional)
- Link to GitHub/GitLab

---

## Phase 2.5: Hot Module Replacement (HMR) Enhancement

### 2.5.1 HMR Status Monitor
- Display real-time HMR connection status in panel header
- Log all HMR events (update, error, full reload)
- Show which files triggered updates with timestamps
- Connection health indicator (connected/reconnecting/offline)

### 2.5.2 State Preservation Layer
- Auto-preserve component state across HMR updates
- Save/restore form inputs, text selections, scroll positions
- Configurable state whitelist/blacklist
- Works with Vite (`import.meta.hot`) and Webpack (`module.hot`)

```tsx
// Usage
<DevSuite hmr={{ preserveState: true, stateWhitelist: ['formData', 'scrollPos'] }}>
  <App />
</DevSuite>
```

### 2.5.3 Error Recovery
- Auto-retry HMR after syntax/build errors are fixed
- Graceful fallback to full page reload when HMR fails
- Display diff of what changed since last successful HMR
- "Reload" button for manual recovery

### 2.5.4 HMR Event API
```ts
// Programmatic access for users
import { devSuite } from 'dev-suite';

devSuite.onHotReload((event) => {
  console.log('HMR:', event.type, event.file);
  // event.type: 'update' | 'error' | 'full-reload'
});

devSuite.preserve('myKey', data);  // Manually preserve data
const data = devSuite.restore('myKey');  // Restore after HMR
```

### 2.5.5 Update History
- Timeline of all HMR updates in current session
- Click to see what changed in each update
- Filter by file type (component, style, util)
- Export session history for debugging

### 2.5.6 Implementation Notes
- Vite: Hook into `import.meta.hot.on('vite:beforeUpdate', ...)`
- Webpack: Hook into `module.hot.addStatusHandler(...)`
- Store preserved state in module-level variable (survives HMR)
- Use WeakMap to track component instances without memory leaks

---

## Phase 3: Integration & Developer Experience

### 3.1 Easy Integration
```tsx
// User's app
import { DevSuite } from 'dev-suite';

function App() {
  return (
    <DevSuite enabled={process.env.NODE_ENV === 'development'}>
      <MyApp />
    </DevSuite>
  );
}
```

### 3.2 Configuration Options
```ts
interface DevSuiteConfig {
  enabled: boolean;
  features: {
    errorLog: boolean;
    colorPalette: boolean;
    components: boolean;
    dataFlow: boolean;
    dataWatch: boolean;
    gitLog: boolean;
    hmr: boolean;
  };
  position: 'left' | 'right' | 'bottom';
  theme: 'light' | 'dark' | 'auto';
  git: {
    enabled: boolean;
    showBranches: boolean;
    maxCommits: number;
  };
  hmr: {
    enabled: boolean;
    preserveState: boolean;
    stateWhitelist?: string[];
    showHistory: boolean;
  };
}
```

### 3.3 Plugin System (Future)
- Allow custom panels/tabs
- Middleware for data interception

---

## Phase 4: Technical Implementation Details

### 4.1 Dependencies
```json
{
  "peerDependencies": {
    "react": ">=17",
    "react-dom": ">=17"
  },
  "dependencies": {
    "framer-motion": "^10.x",      // Animations
    "highlight.js": "^11.x",       // Code highlighting
    "zustand": "^4.x"              // Internal state management
  },
  "devDependencies": {
    "typescript": "^5.x",
    "tsup": "^8.x",                // Bundler
    "react": "^18.x",
    "@types/react": "^18.x"
  }
}
```

### 4.1.1 Additional Dependencies (v1.5+)

**TUI Dependencies:**
```json
{
  "ink": "^4.x",                   // React for CLI
  "ink-select-input": "^4.x",
  "ink-spinner": "^5.x",
  "ink-text-input": "^5.x"
}
```

**DevOps Dependencies (v2.0+):**
```json
{
  "@octokit/rest": "^20.x",        // GitHub API
  "@gitbeaker/rest": "^39.x",      // GitLab API
  "dockerode": "^4.x",             // Docker API
  "@kubernetes/client-node": "^1.x", // Kubernetes
  "simple-git": "^3.x"             // Git operations
}
```

### 4.2 Build Output
- ESM: `dist/index.mjs`
- CJS: `dist/index.js`
- Types: `dist/index.d.ts`
- CSS: `dist/styles.css` (optional extraction)

### 4.3 Git Integration
- Use `simple-git` or shell out to `git` CLI
- Run via Node child_process
- Cache results to avoid performance hit

---

## Phase 5: Testing & Quality

### 5.1 Testing
- Unit tests with Vitest
- Component tests with React Testing Library
- E2E tests (optional) with Playwright

### 5.2 Quality
- ESLint + Prettier
- TypeScript strict mode
- Husky pre-commit hooks
- Conventional commits

---

## Phase 6: Documentation & Publishing

### 6.1 Documentation
- README with quick start
- API reference
- Feature guides
- Demo project/GIFs

### 6.2 Publishing (When Ready)
```bash
# Test locally first
npm link

# In consuming project
npm link dev-suite

# Publish
npm publish --access public
```

---

## Phase 7: Terminal UI (TUI)

A companion CLI interface for developers who prefer terminal-based workflows or need headless operation.

### 7.1 TUI Framework Setup
- Use **Ink** (React for CLI) or **blessed** for rich terminal interfaces
- Shared core logic between React UI and TUI
- Separate entry point: `npx dev-suite tui`

### 7.2 TUI Dashboard
```
┌─────────────────────────────────────────────────────────────┐
│ Dev-Suite Dashboard                          [1-6] Switch  │
├─────────────────────────────────────────────────────────────┤
│ [1] Errors (3)  [2] Tests  [3] Git  [4] Logs  [5] Tasks    │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Recent Errors:                                             │
│  ─────────────────────────────────────────────────────────  │
│  ● TypeError: Cannot read 'x' of undefined                  │
│    src/components/Form.tsx:42                               │
│    2 minutes ago                                            │
│                                                             │
│  ● ReferenceError: process is not defined                   │
│    src/utils/api.ts:15                                      │
│    5 minutes ago                                            │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ Press 'r' to run tests  |  'g' for git menu  |  'q' quit   │
└─────────────────────────────────────────────────────────────┘
```

### 7.3 TUI Features

#### 7.3.1 Error Viewer
- Live error stream from connected dev server
- Stack trace with file:line links (clickable in supported terminals)
- Filter by severity/type
- Clear errors with confirmation

#### 7.3.2 Test Runner Integration
- Run test suites with real-time output
- Test results summary (pass/fail/skip)
- Watch mode for continuous testing
- Coverage reports display

```bash
# CLI usage
dev-suite test              # Run all tests
dev-suite test --watch      # Watch mode
dev-suite test --coverage   # With coverage
dev-suite test src/auth     # Specific path
```

#### 7.3.3 Git Operations
- Quick status, diff, log views
- Interactive staging/commit
- Branch switcher
- Recent files changed

#### 7.3.4 Log Viewer
- Tail application logs
- Filter by level (error, warn, info, debug)
- Search within logs
- Export to file

#### 7.3.5 Task Runner
- Custom npm scripts launcher
- Parallel task execution
- Task history and favorites

### 7.4 TUI Configuration
```ts
// dev-suite.config.ts
export default {
  tui: {
    theme: 'dark' | 'light',
    defaultView: 'errors' | 'tests' | 'git' | 'logs',
    keybindings: {
      runTests: 'r',
      gitMenu: 'g',
      clearErrors: 'c',
    },
  },
};
```

### 7.5 Shared Core Architecture
```
src/
├── core/                    # Shared logic (UI-agnostic)
│   ├── errorCapture.ts
│   ├── testRunner.ts
│   ├── gitClient.ts
│   └── logAggregator.ts
├── react/                   # React UI components
│   └── ...
└── tui/                     # Terminal UI
    ├── index.tsx
    ├── screens/
    │   ├── Dashboard.tsx
    │   ├── Errors.tsx
    │   ├── Tests.tsx
    │   ├── Git.tsx
    │   └── Logs.tsx
    └── components/
        ├── Header.tsx
        ├── Footer.tsx
        └── List.tsx
```

---

## Phase 8: DevOps Suite Integration

Transform dev-suite into a comprehensive DevOps platform connector for full development lifecycle management.

### 8.1 Test Management Hub

#### 8.1.1 Test Type Support
| Test Type | Runners |
|-----------|---------|
| Unit | Jest, Vitest, Mocha |
| Integration | Jest, Vitest |
| E2E | Playwright, Cypress, Selenium |
| Visual | Percy, Chromatic |
| Performance | Lighthouse, k6 |
| API | Postman, REST Client |

#### 8.1.2 Test Dashboard (UI & TUI)
- All test types in single view
- Historical test results
- Flaky test detection
- Test duration trends
- Parallel test execution status

#### 8.1.3 Test Assessment & Reports
```ts
interface TestAssessment {
  summary: {
    total: number;
    passed: number;
    failed: number;
    skipped: number;
    duration: number;
  };
  coverage: {
    lines: number;
    branches: number;
    functions: number;
    statements: number;
  };
  flaky: TestResult[];
  slowest: TestResult[];
  failures: TestFailure[];
}
```

#### 8.1.4 Test Automation
- Pre-commit hooks (run related tests)
- Pre-push validation
- Scheduled test runs
- Auto-retry failed tests
- Test impact analysis (only run affected tests)

### 8.2 CI/CD Platform Connectors

#### 8.2.1 Supported Platforms
| Platform | Integration |
|----------|-------------|
| GitHub Actions | Workflow status, logs, artifacts |
| GitLab CI | Pipelines, jobs, variables |
| Jenkins | Jobs, builds, console output |
| CircleCI | Workflows, pipelines |
| Azure DevOps | Pipelines, releases |
| Bitbucket Pipelines | Pipelines, deployments |

#### 8.2.2 CI/CD Dashboard
```tsx
<DevSuite>
  <CIPanel>
    {/* Real-time pipeline status */}
    <PipelineStatus 
      platform="github"
      repo="owner/repo"
    />
    
    {/* Live build logs */}
    <BuildLogViewer buildId="12345" />
    
    {/* Deployment status */}
    <DeploymentStatus 
      environments={['staging', 'production']}
    />
  </CIPanel>
</DevSuite>
```

#### 8.2.3 CI/CD Features
- Trigger builds/deployments from UI
- View build logs in real-time
- Download artifacts
- Approve/reject deployments
- Environment variable management (secrets-safe)

### 8.3 Cloud Platform Integration

#### 8.3.1 Supported Platforms
| Service | Features |
|---------|----------|
| AWS | Lambda logs, EC2 status, S3, CloudWatch |
| GCP | Cloud Run, GKE, Cloud Functions, Logging |
| Azure | App Service, Functions, Container Apps |
| Vercel | Deployments, functions, logs |
| Netlify | Builds, functions, deploy previews |
| Railway | Services, logs, metrics |
| Render | Services, jobs, logs |

#### 8.3.2 Cloud Dashboard Features
- Resource status overview
- Log streaming from cloud services
- Cost monitoring (optional)
- Environment variable sync
- Deployment history

### 8.4 Container & Orchestration

#### 8.4.1 Docker Integration
```bash
# CLI
dev-suite docker ps           # List containers
dev-suite docker logs <id>    # Stream logs
dev-suite docker build        # Build with progress
dev-suite docker compose up   # Compose with dashboard
```

#### 8.4.2 Kubernetes Integration
- Pod status and logs
- Deployment management
- ConfigMap/Secret viewer
- Event stream
- Resource metrics

### 8.5 Monitoring & Observability

#### 8.5.1 Supported Tools
| Tool | Integration |
|------|-------------|
| Datadog | Metrics, logs, traces |
| New Relic | APM, logs, metrics |
| Sentry | Error tracking, releases |
| LogRocket | Session replay, logs |
| Grafana | Dashboards, alerts |
| Prometheus | Metrics, alerts |

#### 8.5.2 Observability Dashboard
- Unified error tracking across services
- Performance metrics
- Distributed tracing visualization
- Log aggregation from multiple sources
- Alert management

### 8.6 DevOps Configuration

```ts
// dev-suite.config.ts
interface DevOpsConfig {
  ci: {
    platform: 'github' | 'gitlab' | 'jenkins' | 'circleci' | 'azure';
    repo: string;
    watchBranches: string[];
  };
  cloud: {
    provider: 'aws' | 'gcp' | 'azure' | 'vercel' | 'netlify';
    services: string[];
  };
  tests: {
    runners: ('jest' | 'vitest' | 'playwright' | 'cypress')[];
    coverage: boolean;
    watchOnChange: boolean;
  };
  monitoring: {
    sentry?: { dsn: string };
    datadog?: { apiKey: string };
    grafana?: { url: string };
  };
  docker?: {
    composeFile?: string;
  };
  kubernetes?: {
    context: string;
    namespace: string;
  };
}
```

### 8.7 Security Considerations
- Never expose secrets in UI
- Use environment variables for credentials
- Support for secret managers (Vault, AWS Secrets Manager)
- Audit logging for sensitive operations
- Role-based access (future)

### 8.8 Plugin System for DevOps
```ts
// Custom DevOps plugin
interface DevOpsPlugin {
  name: string;
  type: 'ci' | 'cloud' | 'monitoring' | 'test';
  connect: () => Promise<void>;
  getStatus: () => Promise<ServiceStatus>;
  getLogs: (options?: LogOptions) => Promise<LogEntry[]>;
  execute?: (command: string) => Promise<void>;
}

// Register plugin
devSuite.registerPlugin(myCustomPlugin);
```

---

## Updated Project Structure (v2.0)

```
dev-suite/
├── packages/
│   ├── core/                    # Shared logic
│   │   ├── src/
│   │   │   ├── errors/
│   │   │   ├── tests/
│   │   │   ├── git/
│   │   │   ├── logs/
│   │   │   └── devops/
│   │   └── package.json
│   │
│   ├── react/                   # React UI
│   │   ├── src/
│   │   │   ├── components/
│   │   │   └── index.ts
│   │   └── package.json
│   │
│   ├── tui/                     # Terminal UI
│   │   ├── src/
│   │   │   ├── screens/
│   │   │   └── index.tsx
│   │   └── package.json
│   │
│   └── cli/                     # CLI entry point
│       ├── src/
│       │   └── index.ts
│       └── package.json
│
├── package.json                 # Monorepo root
├── turbo.json                   # Turborepo config
└── pnpm-workspace.yaml
```

---

## Updated Implementation Order

### v1.0 - React Dev UI
1. **Week 1**: Project setup + DevSuite wrapper + Panel UI
2. **Week 2**: Error Log + Data Watch
3. **Week 3**: Color Palette + Component Catalog
4. **Week 4**: Git Log + Data Flow
5. **Week 5**: HMR Enhancement
6. **Week 6**: Polish, testing, documentation
7. **Week 7**: Beta release, gather feedback

### v1.5 - TUI Addition
8. **Week 8**: TUI framework setup + shared core extraction
9. **Week 9**: TUI error viewer + log viewer
10. **Week 10**: TUI test runner + git operations
11. **Week 11**: CLI commands + configuration

### v2.0 - DevOps Suite
12. **Week 12**: Monorepo restructure + test management hub
13. **Week 13**: CI/CD connectors (GitHub Actions, GitLab CI)
14. **Week 14**: Cloud platform connectors (Vercel, Netlify, AWS)
15. **Week 15**: Docker + Kubernetes integration
16. **Week 16**: Monitoring integration (Sentry, Datadog)
17. **Week 17**: Plugin system + documentation
18. **Week 18**: v2.0 release

---

## Quick Start Commands

```bash
# Initialize
mkdir dev-suite && cd dev-suite
npm init -y
npm install -D typescript tsup react @types/react
npm install framer-motion zustand highlight.js

# Development
npm run dev       # Watch mode
npm run build     # Production build

# Local testing in another project
npm link
cd ../my-react-app
npm link dev-suite
```

---

## CLI Commands (v1.5+)

```bash
# Start TUI dashboard
dev-suite tui
dev-suite                         # Shorthand

# Error management
dev-suite errors                  # View errors in terminal
dev-suite errors --clear          # Clear all errors
dev-suite errors --watch          # Live error stream

# Test commands
dev-suite test                    # Run all tests
dev-suite test --watch            # Watch mode
dev-suite test --coverage         # With coverage report
dev-suite test:e2e                # Run E2E tests
dev-suite test:visual             # Run visual tests

# Git commands
dev-suite git status              # Quick status
dev-suite git log                 # Recent commits
dev-suite git diff                # Uncommitted changes

# CI/CD commands (v2.0+)
dev-suite ci status               # Pipeline status
dev-suite ci logs <build-id>      # View build logs
dev-suite ci trigger              # Trigger new build
dev-suite deploy staging          # Deploy to staging

# Cloud commands (v2.0+)
dev-suite cloud logs              # Stream cloud logs
dev-suite cloud status            # Service status
dev-suite docker up               # Start containers
dev-suite k8s pods                # List pods

# Configuration
dev-suite init                    # Create config file
dev-suite config                  # View current config
```
