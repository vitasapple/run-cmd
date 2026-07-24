<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
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
};

const STORAGE_KEY = "run-cmd.projects.v1";
const selectedPath = ref("");
const projects = ref<StoredProject[]>([]);
const runningKeys = ref<Set<string>>(new Set());
const busyKey = ref("");
const notice = ref("");
const isDraggingOver = ref(false);
const isSortingInsideApp = ref(false);
let unlistenDragDrop: (() => void) | undefined;
let runningTimer: number | undefined;
let internalDragTimer: number | undefined;

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

onMounted(async () => {
  restoreProjects();
  await wireNativeDrop();
  await refreshRunning();
  runningTimer = window.setInterval(refreshRunning, 1600);
});

onUnmounted(() => {
  unlistenDragDrop?.();
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
    const nextProject: StoredProject = { ...project, scriptOrder };

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
  if (selectedPath.value === project.path) {
    selectedPath.value = projects.value[0]?.path ?? "";
  }
}

async function runScript(script: ScriptInfo) {
  const project = selectedProject.value;
  if (!project) return;

  const key = scriptKey(project.path, script.name);
  busyKey.value = key;
  setNotice("");

  try {
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
        handle=".drag-handle"
        class="project-list"
        ghost-class="ghost"
        animation="160"
        @start="beginInternalDrag"
        @end="endInternalDrag"
      >
        <template #item="{ element }">
          <button
            class="project-item"
            :class="{ active: selectedProject?.path === element.path }"
            type="button"
            @click="selectedPath = element.path"
          >
            <GripVertical class="drag-handle" :size="17" />
            <span class="project-copy">
              <strong>{{ element.name }}</strong>
              <small>{{ shortPath(element.path) }}</small>
            </span>
            <span class="script-total">{{ element.scripts.length }}</span>
          </button>
        </template>
      </draggable>
    </aside>

    <section class="command-pane">
      <div v-if="selectedProject" class="command-view">
        <header class="command-header">
          <div>
            <p class="eyebrow">Commands</p>
            <h2>{{ selectedProject.name }}</h2>
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
          handle=".drag-handle"
          class="command-list"
          ghost-class="ghost"
          animation="160"
          @start="beginInternalDrag"
          @end="endInternalDrag"
        >
          <template #item="{ element }">
            <article class="command-row">
              <GripVertical class="drag-handle" :size="18" />
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
      </div>

      <div v-else class="empty-state">
        <Terminal :size="40" />
        <h2>选择一个前端项目</h2>
        <p>把包含 package.json 的目录拖进窗口，或点击左侧按钮选择目录。</p>
      </div>
    </section>
  </main>
</template>
