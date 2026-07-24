use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
  collections::HashMap,
  fs,
  path::{Path, PathBuf},
  process::{Child, Command, Stdio},
  sync::Mutex,
  thread,
  time::Duration,
};
use tauri::{Manager, State};

#[derive(Default)]
struct ProcessState {
  running: Mutex<HashMap<String, Child>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInfo {
  path: String,
  name: String,
  scripts: Vec<ScriptInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScriptInfo {
  name: String,
  command: String,
}

#[tauri::command]
fn read_project(path: String) -> Result<ProjectInfo, String> {
  let project_path = PathBuf::from(path);
  let package_path = project_path.join("package.json");

  if !package_path.exists() {
    return Err("选择的目录里没有 package.json".to_string());
  }

  let content = fs::read_to_string(&package_path)
    .map_err(|err| format!("读取 package.json 失败: {err}"))?;
  let value: Value = serde_json::from_str(&content)
    .map_err(|err| format!("解析 package.json 失败: {err}"))?;

  let fallback_name = project_path
    .file_name()
    .and_then(|name| name.to_str())
    .unwrap_or("未命名项目")
    .to_string();

  let name = value
    .get("name")
    .and_then(Value::as_str)
    .filter(|name| !name.trim().is_empty())
    .unwrap_or(&fallback_name)
    .to_string();

  let scripts = value
    .get("scripts")
    .and_then(Value::as_object)
    .map(|scripts| {
      scripts
        .iter()
        .filter_map(|(name, command)| {
          command.as_str().map(|command| ScriptInfo {
            name: name.to_string(),
            command: command.to_string(),
          })
        })
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();

  Ok(ProjectInfo {
    path: normalize_path(&project_path),
    name,
    scripts,
  })
}

#[tauri::command]
fn start_script(
  state: State<ProcessState>,
  project_path: String,
  script_name: String,
) -> Result<(), String> {
  let key = process_key(&project_path, &script_name);
  let mut running = state
    .running
    .lock()
    .map_err(|_| "进程状态锁定失败".to_string())?;

  if let Some(child) = running.get_mut(&key) {
    match child.try_wait() {
      Ok(None) => return Err("这个命令已经在运行".to_string()),
      Ok(Some(_)) => {
        running.remove(&key);
      }
      Err(err) => return Err(format!("检查运行状态失败: {err}")),
    }
  }

  let mut command = if cfg!(target_os = "windows") {
    let mut command = Command::new("cmd");
    command.args(["/C", "npm", "run", &script_name]);
    command
  } else {
    let mut command = Command::new("npm");
    command.args(["run", &script_name]);
    command
  };

  prepare_command(&mut command);

  let child = command
    .current_dir(&project_path)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|err| format!("启动命令失败: {err}"))?;

  running.insert(key, child);
  Ok(())
}

#[tauri::command]
fn stop_script(
  state: State<ProcessState>,
  project_path: String,
  script_name: String,
) -> Result<(), String> {
  let key = process_key(&project_path, &script_name);
  let mut running = state
    .running
    .lock()
    .map_err(|_| "进程状态锁定失败".to_string())?;

  let Some(mut child) = running.remove(&key) else {
    return Ok(());
  };

  stop_child_tree(&mut child).map_err(|err| format!("停止命令失败: {err}"))?;
  let _ = child.wait();
  Ok(())
}

#[tauri::command]
fn running_scripts(state: State<ProcessState>) -> Result<Vec<String>, String> {
  let mut running = state
    .running
    .lock()
    .map_err(|_| "进程状态锁定失败".to_string())?;
  let mut finished = Vec::new();

  for (key, child) in running.iter_mut() {
    match child.try_wait() {
      Ok(Some(_)) => finished.push(key.clone()),
      Ok(None) => {}
      Err(_) => finished.push(key.clone()),
    }
  }

  for key in finished {
    running.remove(&key);
  }

  Ok(running.keys().cloned().collect())
}

fn normalize_path(path: &Path) -> String {
  path
    .canonicalize()
    .unwrap_or_else(|_| path.to_path_buf())
    .to_string_lossy()
    .to_string()
}

fn process_key(project_path: &str, script_name: &str) -> String {
  format!("{project_path}::{script_name}")
}

#[cfg(unix)]
fn prepare_command(command: &mut Command) {
  use std::os::unix::process::CommandExt;
  command.process_group(0);
}

#[cfg(windows)]
fn prepare_command(command: &mut Command) {
  use std::os::windows::process::CommandExt;
  const CREATE_NO_WINDOW: u32 = 0x0800_0000;
  const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
  command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn prepare_command(_command: &mut Command) {}

#[cfg(unix)]
fn stop_child_tree(child: &mut Child) -> std::io::Result<()> {
  let pid = child.id() as i32;
  unsafe {
    libc::kill(-pid, libc::SIGTERM);
  }
  for _ in 0..10 {
    if child.try_wait()?.is_some() {
      return Ok(());
    }
    thread::sleep(Duration::from_millis(60));
  }
  unsafe {
    libc::kill(-pid, libc::SIGKILL);
  }
  Ok(())
}

#[cfg(windows)]
fn stop_child_tree(child: &mut Child) -> std::io::Result<()> {
  let mut command = Command::new("taskkill");
  command.args(["/PID", &child.id().to_string(), "/T", "/F"]);
  prepare_command(&mut command);

  let status = command.status()?;

  if status.success() {
    Ok(())
  } else {
    child.kill()
  }
}

#[cfg(not(any(unix, windows)))]
fn stop_child_tree(child: &mut Child) -> std::io::Result<()> {
  child.kill()
}

pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .manage(ProcessState::default())
    .setup(|app| {
      let window = app.get_webview_window("main").expect("main window");
      window.set_focus().ok();
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      read_project,
      start_script,
      stop_script,
      running_scripts
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
