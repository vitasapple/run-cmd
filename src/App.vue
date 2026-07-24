<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import draggable from "vuedraggable";
import {
  FolderPlus,
  GripVertical,
  Play,
  RefreshCw,
  Square,
  Terminal,
  Trash2,
} from "lucide-vue-next";

type ScriptInfo = {
  name: string;
  command: string;
};

type ProjectInfo = {
  path: string;
  name: string;
  scripts: ScriptInfo[];
};

type StoredProject = ProjectInfo & {
  scriptOrder: string[];
  customName?: string;
};

type ScriptLogEvent = {
  key: string;
  stream: string;
  line: string;
};

type ScriptLogLine = ScriptLogEvent & {
  id: number;
};

const STORAGE_KEY = "run-cmd.projects.v1";
const selectedPath = ref("");
const projects = ref<StoredProject[]>([]);
const runningKeys = ref<Set<string>>(new Set());
const scriptLogs = ref<Record<string, ScriptLogLine[]>>({});
const activeLogKey = ref("");
const consoleOutput = ref<HTMLElement | null>(null);
const busyKey = ref("");
const notice = ref("");
const isDraggingOver = ref(false);
const isSortingInsideApp = ref(false);
let unlistenDragDrop: (() => void) | undefined;
let unlistenLog: (() => void) | undefined;
let runningTimer: number | undefined;
let internalDragTimer: number | undefined;
let logId = 0;

const selectedProject = computed(() => {
  return projects.value.find((project) => project.path === selectedPath.value) ?? projects.value[0];
});

const orderedScripts = computed({
  get() {
    const project = selectedProject.value;
    if (!project) return [];

    const byName = new Map(project.scripts.map((script) => [script.name, script]));
    const ordered = project.scriptOrder
      .map((name) => byName.get(name))
      .filter((script): script is ScriptInfo => Boolean(script));
    const rest = project.scripts.filter((script) => !project.scriptOrder.includes(script.name));
    return [...ordered, ...rest];
  },
  set(nextScripts: ScriptInfo[]) {
    const project = selectedProject.value;
    if (!project) return;
    project.scriptOrder = nextScripts.map((script) => script.name);
  },
});

const runningCount = computed(() => runningKeys.value.size);

const activeLogs = computed(() => {
  return activeLogKey.value ? scriptLogs.value[activeLogKey.value] ?? [] : [];
});

const activeLogTitle = computed(() => {
  if (!activeLogKey.value) return "Console";
  const [projectPath, scriptName] = activeLogKey.value.split("::");
  const project = projects.value.find((item) => item.path === projectPath);
  return `${project ? displayProjectName(project) : shortPath(projectPath)} / ${scriptName}`;
});

watch(
  projects,
  (nextProjects) => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(nextProjects));
  },
  { deep: true },
);

watch(selectedProject, (project) => {
  if (project) {
    selectedPath.value = project.path;
  }
});

watch(activeLogs, async () => {
  await nextTick();
  if (consoleOutput.value) {
    consoleOutput.value.scrollTop = consoleOutput.value.scrollHeight;
  }
});

onMounted(async () => {
  restoreProjects();
  await wireNativeDrop();
  await wireScriptLogs();
  await refreshRunning();
  runningTimer = window.setInterval(refreshRunning, 1600);
});

onUnmounted(() => {
  unlistenDragDrop?.();
  unlistenLog?.();
  if (runningTimer) {
    window.clearInterval(runningTimer);
  }
  if (internalDragTimer) {
    window.clearTimeout(internalDragTimer);
  }
});

function restoreProjects() {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return;

  try {
    const stored = JSON.parse(raw) as StoredProject[];
    projects.value = stored.filter((project) => project.path && Array.isArray(project.scripts));
    selectedPath.value = projects.value[0]?.path ?? "";
  } catch {
    localStorage.removeItem(STORAGE_KEY);
  }
}

async function wireNativeDrop() {
  const webview = getCurrentWebview();
  unlistenDragDrop = await webview.onDragDropEvent(async (event) => {
    if (isSortingInsideApp.value) {
      isDraggingOver.value = false;
      return;
    }

    if (event.payload.type === "over") {
      isDraggingOver.value = true;
    }

    if (event.payload.type === "drop") {
      isDraggingOver.value = false;
      const paths = event.payload.paths ?? [];
      for (const path of paths) {
        await addProject(path);
      }
    }

    if (event.payload.type === "leave") {
      isDraggingOver.value = false;
    }
  });
}

async function wireScriptLogs() {
  unlistenLog = await listen<ScriptLogEvent>("script-log", (event) => {
    appendScriptLog(event.payload);
  });
}

function beginInternalDrag() {
  if (internalDragTimer) {
    window.clearTimeout(internalDragTimer);
  }
  isSortingInsideApp.value = true;
  isDraggingOver.value = false;
}

function endInternalDrag() {
  isDraggingOver.value = false;
  internalDragTimer = window.setTimeout(() => {
    isSortingInsideApp.value = false;
  }, 250);
}

async function chooseProject() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择前端项目目录",
  });

  if (typeof selected === "string") {
    await addProject(selected);
  }
}

async function addProject(path: string) {
  setNotice("");
  try {
    const project = await invoke<ProjectInfo>("read_project", { path });
    const existing = projects.value.find((item) => item.path === project.path);
    const scriptOrder = mergeScriptOrder(existing?.scriptOrder ?? [], project.scripts);
    const nextProject: StoredProject = {
      ...project,
      customName: existing?.customName,
      scriptOrder,
    };

    if (existing) {
      Object.assign(existing, nextProject);
    } else {
      projects.value.push(nextProject);
    }

    selectedPath.value = project.path;
  } catch (error) {
    setNotice(String(error));
  }
}

async function reloadProject(project: StoredProject) {
  await addProject(project.path);
}

function removeProject(project: StoredProject) {
  projects.value = projects.value.filter((item) => item.path !== project.path);
  removeProjectLogs(project.path);
  if (selectedPath.value === project.path) {
    selectedPath.value = projects.value[0]?.path ?? "";
  }
}

function renameProject(project: StoredProject, event: Event) {
  const value = (event.target as HTMLInputElement).value.trim();
  project.customName = value || undefined;
}

async function runScript(script: ScriptInfo) {
  const project = selectedProject.value;
  if (!project) return;

  const key = scriptKey(project.path, script.name);
  busyKey.value = key;
  setNotice("");

  try {
    activeLogKey.value = key;
    setScriptLog(key, {
      key,
      stream: "system",
      line: `> npm run ${script.name}`,
    });
    await invoke("start_script", {
      projectPath: project.path,
      scriptName: script.name,
    });
    runningKeys.value = new Set([...runningKeys.value, key]);
  } catch (error) {
    setNotice(String(error));
  } finally {
    busyKey.value = "";
  }
}

async function stopScript(script: ScriptInfo) {
  const project = selectedProject.value;
  if (!project) return;

  const key = scriptKey(project.path, script.name);
  busyKey.value = key;

  try {
    appendScriptLog({
      key,
      stream: "system",
      line: "> stop requested",
    });
    await invoke("stop_script", {
      projectPath: project.path,
      scriptName: script.name,
    });
    const next = new Set(runningKeys.value);
    next.delete(key);
    runningKeys.value = next;
  } catch (error) {
    setNotice(String(error));
  } finally {
    busyKey.value = "";
  }
}

async function refreshRunning() {
  try {
    const keys = await invoke<string[]>("running_scripts");
    runningKeys.value = new Set(keys);
  } catch {
    runningKeys.value = new Set();
  }
}

function scriptKey(projectPath: string, scriptName: string) {
  return `${projectPath}::${scriptName}`;
}

function isRunning(script: ScriptInfo) {
  const project = selectedProject.value;
  return Boolean(project && runningKeys.value.has(scriptKey(project.path, script.name)));
}

function mergeScriptOrder(order: string[], scripts: ScriptInfo[]) {
  const scriptNames = scripts.map((script) => script.name);
  return [...order.filter((name) => scriptNames.includes(name)), ...scriptNames.filter((name) => !order.includes(name))];
}

function displayProjectName(project: StoredProject) {
  return project.customName || project.name;
}

function setScriptLog(key: string, line: ScriptLogEvent) {
  scriptLogs.value = {
    ...scriptLogs.value,
    [key]: [{ ...line, id: ++logId }],
  };
}

function appendScriptLog(line: ScriptLogEvent) {
  const current = scriptLogs.value[line.key] ?? [];
  scriptLogs.value = {
    ...scriptLogs.value,
    [line.key]: [...current, { ...line, id: ++logId }].slice(-500),
  };
}

function clearActiveLog() {
  if (!activeLogKey.value) return;
  scriptLogs.value = {
    ...scriptLogs.value,
    [activeLogKey.value]: [],
  };
}

function removeProjectLogs(projectPath: string) {
  const nextLogs = { ...scriptLogs.value };
  for (const key of Object.keys(nextLogs)) {
    if (key.startsWith(`${projectPath}::`)) {
      delete nextLogs[key];
    }
  }
  scriptLogs.value = nextLogs;
  if (activeLogKey.value.startsWith(`${projectPath}::`)) {
    activeLogKey.value = "";
  }
}

function setNotice(message: string) {
  notice.value = message;
}

function shortPath(path: string) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.slice(-3).join("/");
}
</script>

<template>
  <main class="app-shell" :class="{ 'is-dragging': isDraggingOver }">
    <aside class="project-pane">
      <div class="pane-header">
        <div>
          <p class="eyebrow">Projects</p>
          <h1>Run Cmd</h1>
        </div>
        <button class="icon-button primary" type="button" title="添加项目" @click="chooseProject">
          <FolderPlus :size="19" />
        </button>
      </div>

      <button class="drop-zone" type="button" @click="chooseProject">
        <Terminal :size="20" />
        <span>选择或拖入项目目录</span>
      </button>

      <draggable
        v-model="projects"
        item-key="path"
        tag="div"
        handle=".drag-handle"
        class="project-list"
        ghost-class="ghost"
        chosen-class="chosen"
        drag-class="dragging-item"
        animation="160"
        :force-fallback="true"
        :fallback-on-body="true"
        @start="beginInternalDrag"
        @end="endInternalDrag"
      >
        <template #item="{ element }">
          <div
            class="project-item"
            :class="{ active: selectedProject?.path === element.path }"
          >
            <span class="drag-handle" title="拖动排序">
              <GripVertical :size="17" />
            </span>
            <button class="project-select" type="button" @click="selectedPath = element.path">
              <span class="project-copy">
                <strong>{{ displayProjectName(element) }}</strong>
                <small>{{ shortPath(element.path) }}</small>
              </span>
            </button>
            <span class="script-total">{{ element.scripts.length }}</span>
            <button class="project-remove" type="button" title="从列表移除项目" @click.stop="removeProject(element)">
              <Trash2 :size="15" />
            </button>
          </div>
        </template>
      </draggable>
    </aside>

    <section class="command-pane">
      <div v-if="selectedProject" class="command-view">
        <header class="command-header">
          <div>
            <p class="eyebrow">Commands</p>
            <input
              class="project-name-input"
              type="text"
              :value="displayProjectName(selectedProject)"
              title="编辑项目显示名"
              @input="renameProject(selectedProject, $event)"
            />
            <p class="path-line">{{ selectedProject.path }}</p>
          </div>
          <div class="header-actions">
            <span v-if="runningCount" class="running-pill">{{ runningCount }} running</span>
            <button class="icon-button" type="button" title="刷新 package.json" @click="reloadProject(selectedProject)">
              <RefreshCw :size="18" />
            </button>
            <button class="icon-button danger" type="button" title="移除项目" @click="removeProject(selectedProject)">
              <Trash2 :size="18" />
            </button>
          </div>
        </header>

        <p v-if="notice" class="notice">{{ notice }}</p>

        <draggable
          v-model="orderedScripts"
          item-key="name"
          tag="div"
          handle=".drag-handle"
          class="command-list"
          ghost-class="ghost"
          chosen-class="chosen"
          drag-class="dragging-item"
          animation="160"
          :force-fallback="true"
          :fallback-on-body="true"
          @start="beginInternalDrag"
          @end="endInternalDrag"
        >
          <template #item="{ element }">
            <article class="command-row">
              <span class="drag-handle" title="拖动排序">
                <GripVertical :size="18" />
              </span>
              <div class="command-copy">
                <strong>{{ element.name }}</strong>
                <code>{{ element.command }}</code>
              </div>
              <button
                v-if="isRunning(element)"
                class="run-button stop"
                type="button"
                :disabled="busyKey === scriptKey(selectedProject.path, element.name)"
                @click="stopScript(element)"
              >
                <Square :size="17" />
                <span>停止</span>
              </button>
              <button
                v-else
                class="run-button"
                type="button"
                :disabled="busyKey === scriptKey(selectedProject.path, element.name)"
                @click="runScript(element)"
              >
                <Play :size="17" />
                <span>运行</span>
              </button>
            </article>
          </template>
        </draggable>

        <div v-if="!selectedProject.scripts.length" class="empty-state compact">
          <Terminal :size="30" />
          <p>这个 package.json 里还没有 scripts。</p>
        </div>

        <section class="console-panel">
          <div class="console-header">
            <div>
              <p class="eyebrow">Console</p>
              <strong>{{ activeLogTitle }}</strong>
            </div>
            <button class="console-clear" type="button" :disabled="!activeLogs.length" @click="clearActiveLog">
              清空
            </button>
          </div>
          <div ref="consoleOutput" class="console-output" aria-live="polite">
            <p v-if="!activeLogs.length" class="console-empty">点击运行后会在这里显示输出日志。</p>
            <p v-for="line in activeLogs" v-else :key="line.id" :class="['console-line', line.stream]">
              <span>{{ line.line }}</span>
            </p>
          </div>
        </section>
      </div>

      <div v-else class="empty-state">
        <Terminal :size="40" />
        <h2>选择一个前端项目</h2>
        <p>把包含 package.json 的目录拖进窗口，或点击左侧按钮选择目录。</p>
      </div>
    </section>
  </main>
</template>
