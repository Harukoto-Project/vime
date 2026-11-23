use std::{io, path::Path};

use eframe::egui::{self, RichText, Ui};
use vime::{
    i18n::{tr, tr_args},
    ime::Language,
};

use crate::keyboard::{LearnedTypo, load_learned_typos};

use super::{ERROR, SECONDARY, panel};

#[derive(Default)]
pub(super) struct LearnedTypos {
    revision: Option<u64>,
    entries: Vec<LearnedTypo>,
    error: Option<io::Error>,
    search: String,
    language: Option<Language>,
    /// The filter for which deleting every shown entry awaits confirmation.
    confirming: Option<(String, Option<Language>)>,
    delete_error: Option<io::Error>,
}

impl LearnedTypos {
    pub(super) fn refresh_on_open(&mut self) {
        self.revision = None;
    }

    fn update(&mut self, directory: &Path, revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        match load_learned_typos(directory) {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(error) => {
                self.entries.clear();
                self.error = Some(error);
            }
        }
    }

    pub(super) fn delete_failed(&mut self, error: io::Error) {
        self.delete_error = Some(error);
        self.revision = None;
    }

    /// Shows the entries and returns the ones the user deleted.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        directory: &Path,
        revision: u64,
    ) -> Vec<LearnedTypo> {
        self.update(directory, revision);
        let mut removed = Vec::new();
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading(tr("学習済みの誤字"));
            ui.add_space(8.);
            if let Some(error) = &self.delete_error {
                ui.label(
                    RichText::new(tr_args(
                        "学習済みの誤字を削除できませんでした ({0})",
                        &[&error.to_string()],
                    ))
                    .color(ERROR),
                );
            }
            if let Some(error) = &self.error {
                ui.label(
                    RichText::new(tr_args(
                        "学習済みの誤字を読み込めませんでした ({0})",
                        &[&error.to_string()],
                    ))
                    .color(ERROR),
                );
                return;
            }
            if self.entries.is_empty() {
                ui.label(RichText::new(tr("学習済みの誤字はまだありません")).color(SECONDARY));
                return;
            }

            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text(tr("誤字・修正後の語を検索"))
                        .desired_width(230.),
                );
                ui.label(tr("言語"));
                egui::ComboBox::from_id_salt("learned_typo_language")
                    .selected_text(
                        self.language
                            .map_or_else(|| tr("すべての言語"), Language::label),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.language, None, tr("すべての言語"));
                        for language in Language::ALL {
                            ui.selectable_value(
                                &mut self.language,
                                Some(language),
                                language.label(),
                            );
                        }
                    });
            });
            let query = self.search.trim().to_lowercase();
            let filter = (query.clone(), self.language);
            if self.confirming.as_ref() != Some(&filter) {
                self.confirming = None;
            }
            let rows: Vec<_> = self
                .entries
                .iter()
                .filter(|entry| {
                    self.language
                        .is_none_or(|language| entry.language == language.label())
                        && (query.is_empty()
                            || entry.original.to_lowercase().contains(&query)
                            || entry.replacement.to_lowercase().contains(&query))
                })
                .collect();
            ui.add_space(6.);
            if rows.is_empty() {
                ui.label(RichText::new(tr("条件に一致する誤字はありません")).color(SECONDARY));
                return;
            }
            let count = rows.len().to_string();
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(tr_args("表示 {0} 件", &[&count]))
                        .small()
                        .color(SECONDARY),
                );
                if self.confirming.is_none() {
                    if ui
                        .small_button(tr_args("表示中の{0}件を削除", &[&count]))
                        .clicked()
                    {
                        self.confirming = Some(filter.clone());
                    }
                } else {
                    if ui
                        .small_button(
                            RichText::new(tr_args("{0}件を削除する", &[&count])).color(ERROR),
                        )
                        .clicked()
                    {
                        removed.extend(rows.iter().map(|entry| (*entry).clone()));
                        self.confirming = None;
                    }
                    if ui.small_button(tr("キャンセル")).clicked() {
                        self.confirming = None;
                    }
                }
            });
            ui.add_space(3.);
            let column = (ui.available_width() - 2. * ui.spacing().item_spacing.x) / 3.;
            egui::Grid::new("learned_typo_rows")
                .num_columns(3)
                .min_col_width(column)
                .max_col_width(column)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong(tr("誤字"));
                    ui.strong(tr("修正後"));
                    ui.strong(tr("言語"));
                    ui.end_row();
                    for entry in rows {
                        ui.add(egui::Label::new(&entry.original).wrap());
                        ui.add(egui::Label::new(&entry.replacement).wrap());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&entry.language).color(SECONDARY));
                            if ui.small_button(tr("削除")).clicked() {
                                removed.push(entry.clone());
                            }
                        });
                        ui.end_row();
                    }
                });
        });
        if !removed.is_empty() {
            self.delete_error = None;
            self.confirming = None;
            self.entries.retain(|entry| !removed.contains(entry));
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn opens_existing_entries_and_refreshes_after_learning() {
        let directory = tempfile::tempdir().unwrap();
        let mut page = LearnedTypos::default();
        page.update(directory.path(), 0);
        assert!(page.entries.is_empty());
        assert!(page.error.is_none());

        fs::write(
            directory.path().join("typos.json"),
            r#"[{"language":"日本語","original":"こんいちは","replacement":"こんにちは"}]"#,
        )
        .unwrap();
        page.update(directory.path(), 1);
        assert_eq!(page.entries[0].original, "こんいちは");
        assert_eq!(page.entries[0].replacement, "こんにちは");

        let context = egui::Context::default();
        let render = |page: &mut LearnedTypos| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480., 360.));
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    let rect = ui
                        .scope(|ui| page.show(ui, directory.path(), 1))
                        .response
                        .rect;
                    assert!(
                        rect.right() <= screen.right(),
                        "list exceeded the narrow layout"
                    );
                },
            )
        };
        let shows_replacement = |mut output: egui::FullOutput| {
            output.textures_delta.clear();
            output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "こんにちは")
        })
        };
        assert!(shows_replacement(render(&mut page)));
        page.language = Some(Language::English);
        assert!(!shows_replacement(render(&mut page)));

        fs::write(directory.path().join("typos.json"), "invalid json").unwrap();
        page.refresh_on_open();
        page.update(directory.path(), 1);
        assert!(page.entries.is_empty());
        assert!(page.error.is_some());
    }
}
