//! 休闲时光 — 个人娱乐库（漫画 / 视频 / 小说[预留]）
//!
//! 单数据库分表存储，模块化命令层，前端统一壳导航。
//!
//! 双端支持：桌面窗口（Tauri IPC）+ 浏览器/局域网（HTTP 服务，见 http 模块）。
//! 命令层不直接依赖 AppHandle 参数，改用全局状态（app_handle / app_data_dir），
//! 同一套命令逻辑可同时被 tauri::command 与 HTTP handler 调用。
mod commands;
mod covers;
mod covers_db;
mod db;
mod events;
mod http;
mod novel_epub;
mod novel_txt;
mod scanner;
mod story_covers;
mod story_first_page;
mod story_pages;
mod story_scanner;
mod video_covers;
mod video_scanner;

use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::Manager;

/// 全局 AppHandle（setup 时设置；事件转发与 HTTP 服务共用）
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
/// 全局数据目录（app_data_dir；命令层不再依赖 AppHandle 参数）
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 全局 AppHandle（HTTP 模式仍运行于 Tauri 应用内，始终可用）
pub fn app_handle() -> &'static tauri::AppHandle {
    APP.get().expect("AppHandle 未初始化")
}

/// 全局数据目录
pub fn app_data_dir() -> &'static PathBuf {
    DATA_DIR.get().expect("数据目录未初始化")
}

#[tauri::command]
fn get_data_directory() -> String {
    crate::app_data_dir().to_string_lossy().to_string()
}

#[tauri::command]
fn get_install_directory() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

#[tauri::command]
fn open_directory(path: String) -> Result<String, String> {
    opener::open(&path).map_err(|e| format!("打开目录失败: {}", e))?;
    Ok(path)
}

#[tauri::command]
fn check_path_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 本机局域网访问地址列表 + 访问口令（仅桌面窗口展示用，不暴露给浏览器 API）
#[tauri::command]
fn get_web_urls() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "urls": crate::http::web_urls(),
        "password": crate::http::web_password(),
    }))
}

/// 在系统默认浏览器中打开链接（桌面窗口点击局域网访问链接用）
#[tauri::command]
fn open_web_url(url: String) -> Result<(), String> {
    opener::open(&url).map_err(|e| format!("打开浏览器失败: {e}"))
}

#[tauri::command]
fn set_window_pin(app: tauri::AppHandle, pin: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_always_on_top(pin)
            .map_err(|e| format!("设置置顶失败: {}", e))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(appkit_core::tauri_bridge::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .setup(|app| {
            log::info!("休闲时光启动成功");

            // 初始化配置存储目录（app_data_dir）
            if let Ok(dir) = app.path().app_data_dir() {
                // appkit-core 通用命令（list_data_files / get_db_tables 等）依赖配置目录，
                // 未设置时调用会 panic；休闲时光不使用其 config.json，指向 app_data_dir 即可
                appkit_core::config::store::set_config_dir(dir.clone());
                // 全局状态：命令层与 HTTP 服务共用（不依赖 AppHandle 参数）
                let _ = APP.set(app.handle().clone());
                let _ = DATA_DIR.set(dir.clone());
                // 初始化数据库（单库分表：漫画 / 视频 / 小说[预留]）
                if let Err(e) = db::init_db(&dir.join("leisure.db")) {
                    eprintln!("[FATAL] 数据库初始化失败: {e}");
                    log::error!("数据库初始化失败: {e}");
                }
            }
            // 启动 HTTP 服务：本机/局域网浏览器访问（口令保护，见 http 模块）
            http::start();

            // 编译时嵌入 icon.png，运行时解码为窗口图标
            if let Some(window) = app.get_webview_window("main") {
                let png_bytes = include_bytes!("../icons/icon.png");
                match image::load_from_memory(png_bytes) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
                        if let Err(e) = window.set_icon(icon) {
                            eprintln!("[setup] set_icon error: {:?}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("[setup] decode icon error: {:?}", e);
                    }
                }

                #[cfg(debug_assertions)]
                if let Ok(title) = window.title() {
                    let _ = window.set_title(&format!("{} [开发版]", title));
                }
            }

            // 系统托盘：关闭窗口隐藏到托盘常驻，小图标唤出（参照 psman 同款实现）
            let show = tauri::menu::MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            tauri::tray::TrayIconBuilder::with_id("main")
                .tooltip("休闲时光")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // 左键单击：切换主窗口显隐（只响应“抬起”，防止点击被拆成 Down/Up 两次触发）
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            match w.is_visible() {
                                Ok(true) => {
                                    let _ = w.hide();
                                }
                                _ => {
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭按钮 → 隐藏到托盘常驻（进程不退出，HTTP 局域网服务继续），托盘菜单/左键单击唤出
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_directory,
            check_path_exists,
            get_app_version,
            get_web_urls,
            open_web_url,
            get_data_directory,
            get_install_directory,
            set_window_pin,
            // ── 漫画书架（commands_comics）──
            commands::commands_comics::add_root_dir,
            commands::commands_comics::remove_root_dir,
            commands::commands_comics::get_root_dirs,
            commands::commands_comics::scan_root_dir,
            commands::commands_comics::rescan_comic,
            commands::commands_comics::rescan_all,
            commands::commands_comics::clear_cover_cache,
            commands::commands_comics::get_top_level_comics,
            commands::commands_comics::get_chapters,
            commands::commands_comics::get_comic,
            commands::commands_comics::delete_comic,
            commands::commands_comics::rename_comic,
            commands::commands_comics::get_series_progress,
            commands::commands_comics::get_all_series_progress,
            commands::commands_comics::open_comic_directory,
            commands::commands_comics::sync_index,
            commands::commands_comics::get_completed_status,
            commands::commands_comics::set_completed_status,
            commands::commands_comics::get_all_completed_status,
            commands::commands_comics::get_comic_cover_data_url,
            commands::commands_comics::list_pages,
            commands::commands_comics::get_page_data_url,
            commands::commands_comics::get_comic_page_index,
            commands::commands_comics::get_page_preview_data_url,
            commands::commands_comics::set_comic_cover_from_offset,
            commands::commands_comics::reset_comic_cover,
            commands::commands_comics::get_comic_pdf_data,
            commands::commands_comics::upload_comic_cover,
            commands::commands_comics::set_reading_progress,
            commands::commands_comics::get_reading_progress,
            commands::commands_comics::get_comic_config,
            commands::commands_comics::set_comic_config,
            commands::commands_comics::batch_set_sort_order,
            // ── 视频管理（commands_videos）──
            commands::commands_videos::scan_directory,
            commands::commands_videos::add_video,
            commands::commands_videos::list_videos,
            commands::commands_videos::get_video,
            commands::commands_videos::get_video_subtitle,
            commands::commands_videos::update_video,
            commands::commands_videos::update_video_media_meta,
            commands::commands_videos::get_video_roots,
            commands::commands_videos::add_video_root,
            commands::commands_videos::remove_video_root,
            commands::commands_videos::rescan_video_root,
            commands::commands_videos::rescan_all_video_roots,
            commands::commands_videos::delete_video,
            commands::commands_videos::batch_set_video_sort_order,
            commands::commands_videos::get_video_play_path,
            commands::commands_videos::open_video_folder,
            commands::commands_videos::get_video_cover_data_url,
            commands::commands_videos::has_video_cover,
            commands::commands_videos::upload_cover,
            // ── 演员/标签/剧集（commands_series）──
            commands::commands_series::list_actors,
            commands::commands_series::save_actor,
            commands::commands_series::delete_actor,
            commands::commands_series::upload_actor_image,
            commands::commands_series::get_actor_image_data_url,
            commands::commands_series::list_tag_groups,
            commands::commands_series::save_tag_group,
            commands::commands_series::delete_tag_group,
            commands::commands_series::list_tags,
            commands::commands_series::save_tag,
            commands::commands_series::delete_tag,
            commands::commands_series::list_series,
            commands::commands_series::get_series,
            commands::commands_series::create_series,
            commands::commands_series::create_series_from_dir,
            commands::commands_series::update_series,
            commands::commands_series::delete_series,
            commands::commands_series::set_videos_series,
            commands::commands_series::update_video_progress,
            // ── 小说（commands_novels）──
            commands::commands_novels::add_novel_root,
            commands::commands_novels::rescan_novel_root,
            commands::commands_novels::remove_novel_root,
            commands::commands_novels::get_novel_roots,
            commands::commands_novels::add_novel,
            commands::commands_novels::list_novels,
            commands::commands_novels::get_novel,
            commands::commands_novels::delete_novel,
            commands::commands_novels::rename_novel,
            commands::commands_novels::get_novel_cover_data_url,
            commands::commands_novels::get_novel_chapter_content,
            commands::commands_novels::set_novel_progress,
            commands::commands_novels::open_novel_folder,
            // ── 故事会（commands_storyclub）──
            commands::commands_storyclub::storyclub_set_root,
            commands::commands_storyclub::storyclub_get_root,
            commands::commands_storyclub::storyclub_remove_root,
            commands::commands_storyclub::storyclub_rescan,
            commands::commands_storyclub::storyclub_list_issues,
            commands::commands_storyclub::storyclub_get_pdf_data,
            commands::commands_storyclub::storyclub_get_pdf_path,
            commands::commands_storyclub::storyclub_set_progress,
            commands::commands_storyclub::storyclub_update_page_count,
            commands::commands_storyclub::storyclub_open_folder,
            commands::commands_storyclub::storyclub_open_year_dir,
            commands::commands_storyclub::storyclub_get_cover_data_url,
            commands::commands_storyclub::storyclub_get_covers_batch,
            commands::commands_storyclub::storyclub_upload_cover,
            commands::commands_storyclub::storyclub_first_page_jpeg,
            commands::commands_storyclub::storyclub_missing_covers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
