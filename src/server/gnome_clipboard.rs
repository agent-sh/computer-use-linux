use super::*;
use crate::remote_desktop::{
    press_keycode_chord, start_portal_keyboard_clipboard_session, ClipboardSelection,
    PortalClipboardSession,
};

pub(super) fn atspi_outcome_allows_clipboard_fallback(insert: &Result<FocusedTextInsert>) -> bool {
    matches!(
        insert,
        Ok(FocusedTextInsert::NotEditable(_))
            | Ok(FocusedTextInsert::EditFailed {
                may_have_changed: false,
                ..
            })
            | Err(_)
    )
}

impl ComputerUseLinux {
    pub(super) async fn type_text_through_gnome_clipboard(
        &self,
        text: &str,
        portal_target: &WindowTarget,
        focus: Option<WindowFocusResult>,
        received: Option<serde_json::Value>,
        input_guard: InputOperationGuard,
        refusal: String,
    ) -> ActionOutput {
        let clipboard_guard = Arc::clone(&self.kde_clipboard_lock).lock_owned().await;
        let cached_slot = Arc::clone(&self.portal_clipboard_session);
        let server = self.clone();
        let portal_target = portal_target.clone();
        let text = text.to_string();
        let portal_operation_guard = input_guard.clone();
        let (guards, guarded_result) =
            run_cancellation_safe_guarded((input_guard, clipboard_guard), async move {
                let mut slot = cached_slot.lock().await;
                let mut started_fresh = false;
                let need_new = match slot.as_ref() {
                    Some(existing) if existing.is_valid() => false,
                    Some(_) => {
                        if let Some(old) = slot.take() {
                            old.close().await;
                        }
                        true
                    }
                    None => true,
                };
                if need_new {
                    let session =
                        start_portal_keyboard_clipboard_session()
                            .await
                            .map_err(|error| {
                                (
                                    focus.clone(),
                                    format!(
                                    "{refusal} GNOME clipboard paste could not start: {error:#}"
                                ),
                                )
                            })?;
                    *slot = Some(session);
                    started_fresh = true;
                }

                // Re-focus after portal startup in case a first-time consent dialog stole focus.
                let focus = if started_fresh || focus.is_none() {
                    match server.focus_target_for_input(&portal_target).await {
                        Ok(refreshed) => refreshed,
                        Err(error) => return Err((focus, error)),
                    }
                } else {
                    focus
                };
                let paste_shortcut =
                    kde_clipboard_paste_shortcut(&portal_target, focus.as_ref()).await;

                let session = slot
                    .as_mut()
                    .expect("portal clipboard session was just populated");
                let outcome = run_gnome_clipboard_paste_text(
                    session,
                    &text,
                    paste_shortcut,
                    portal_operation_guard,
                )
                .await;

                let keep_session = session.is_valid() && outcome.is_ok();
                if !keep_session {
                    if let Some(broken) = slot.take() {
                        broken.close().await;
                    }
                }

                match outcome {
                    Ok(message) => Ok((focus, message)),
                    Err(error) => Err((focus, error)),
                }
            })
            .await;
        let Some((_input_guard, _clipboard_guard)) = guards else {
            return action_result_with_focus(
                "type_text",
                Err(guarded_result.unwrap_err()),
                received,
                None,
            );
        };
        match guarded_result.expect("guarded task returned its guards") {
            Ok((focus, message)) => {
                let notes = self.input_landing_notes(focus.as_ref(), true).await;
                with_notes(
                    successful_action_with_focus("type_text", &message, received, focus),
                    notes,
                )
            }
            Err((focus, message)) => {
                action_result_with_focus("type_text", Err(message), received, focus)
            }
        }
    }
}

async fn run_gnome_clipboard_paste_text(
    session: &mut PortalClipboardSession,
    text: &str,
    paste_shortcut: KdeClipboardPasteShortcut,
    operation_guard: InputOperationGuard,
) -> std::result::Result<String, String> {
    let previous = match session.read_current_selection().await {
        Ok(previous) => previous,
        Err(error) => {
            return Err(format!(
                "GNOME clipboard paste was not used; the current clipboard could not be preserved: {error:#}"
            ));
        }
    };
    if let Err(error) = session
        .replace_selection(ClipboardSelection::text(text))
        .await
    {
        let restore_error = restore_gnome_clipboard(session, &previous).await;
        return Err(match restore_error {
            Some(restore_error) => format!(
                "GNOME clipboard replacement failed ambiguously; restoring the previous contents also failed, and text was not replayed: {error:#}; restore failed: {restore_error:#}"
            ),
            None => format!(
                "GNOME clipboard replacement failed ambiguously and the previous contents were restored; text was not replayed: {error:#}"
            ),
        });
    }

    let (modifiers, keycode) = kde_clipboard_paste_chord(paste_shortcut);
    let paste_result = press_keycode_chord(session.keyboard(), modifiers, keycode, operation_guard)
        .await
        .map_err(|error| format!("{error:#}"));
    sleep(kde_clipboard_restore_delay(text)).await;

    let still_owned = session.selection_still_owned();
    let restore_error = if still_owned {
        restore_gnome_clipboard(session, &previous).await
    } else {
        None
    };

    let ownership_note = if still_owned {
        String::new()
    } else {
        " A newer clipboard owner was left unchanged.".to_string()
    };
    match (paste_result, restore_error) {
        (Ok(()), None) => Ok(format!(
            "Action pasted through the GNOME portal clipboard.{ownership_note}"
        )),
        (Ok(()), Some(error)) => Ok(format!(
            "Action pasted through the GNOME portal clipboard. Warning: previous clipboard contents could not be restored: {error:#}"
        )),
        (Err(error), None) => Err(format!(
            "{error}; previous clipboard contents were restored; text was not replayed{ownership_note}"
        )),
        (Err(error), Some(restore_error)) => Err(format!(
            "{error}; previous clipboard contents could not be restored: {restore_error:#}; text was not replayed"
        )),
    }
}

async fn restore_gnome_clipboard(
    session: &mut PortalClipboardSession,
    previous: &ClipboardSelection,
) -> Option<anyhow::Error> {
    if !session.selection_still_owned() {
        return None;
    }
    session.replace_selection(previous.clone()).await.err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_fallback_only_follows_no_edit_atspi_outcomes() {
        assert!(atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::NotEditable("no AT-SPI window".to_string())
        )));
        assert!(atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::EditFailed {
                reason: "rejected before mutation".to_string(),
                may_have_changed: false,
            }
        )));
        assert!(atspi_outcome_allows_clipboard_fallback(&Err(
            anyhow::anyhow!("AT-SPI bus unavailable")
        )));

        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::PasswordField
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::PasswordUnverifiable
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::ReadbackFailed
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::ReadbackTimedOut
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::Mismatch {
                expected_count: 4,
                read_count: 2,
                read_text: "ab".to_string(),
                read_truncated: false,
            }
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::EditFailed {
                reason: "insert timed out".to_string(),
                may_have_changed: true,
            }
        )));
        assert!(!atspi_outcome_allows_clipboard_fallback(&Ok(
            FocusedTextInsert::Verified {
                role: "text".to_string(),
                replaced_selection: false,
                caret_after_text: true,
            }
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_portal_clipboard_paste_restores_rich_clipboard() {
        if std::env::var_os("CUL_LIVE_PORTAL_TEST").is_none() {
            return;
        }
        let expect_empty = std::env::var_os("CUL_LIVE_PORTAL_TEST_EMPTY").is_some();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let mut session = start_portal_keyboard_clipboard_session().await.unwrap();
        let first = run_gnome_clipboard_paste_text(
            &mut session,
            "你好！这是一段测试文字。",
            KdeClipboardPasteShortcut::Standard,
            InputOperationGuard::new(Arc::clone(&lock).lock_owned().await),
        )
        .await
        .unwrap();
        assert!(first.contains("pasted through the GNOME portal clipboard"));

        // Second paste reuses the same portal clipboard session without re-reading its own selection.
        let second = run_gnome_clipboard_paste_text(
            &mut session,
            "第二段",
            KdeClipboardPasteShortcut::Standard,
            InputOperationGuard::new(Arc::clone(&lock).lock_owned().await),
        )
        .await
        .unwrap();
        assert!(second.contains("pasted through the GNOME portal clipboard"));

        if expect_empty {
            let types = tokio::process::Command::new("wl-paste")
                .arg("--list-types")
                .output()
                .await
                .unwrap();
            assert!(
                !types.status.success() || types.stdout.is_empty(),
                "expected empty clipboard, got: {}",
                String::from_utf8_lossy(&types.stdout)
            );
        } else {
            let restored = tokio::process::Command::new("wl-paste")
                .args(["--no-newline", "--type", "text/html"])
                .output()
                .await
                .unwrap();
            assert!(
                restored.status.success(),
                "wl-paste failed: {}",
                String::from_utf8_lossy(&restored.stderr)
            );
            assert_eq!(restored.stdout, b"<b>previous clipboard</b>");
        }
        session.close().await;
    }
}
