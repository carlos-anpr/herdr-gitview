//! Cálculo de carriles del grafo estilo Git Graph (VS Code). PROPIETARIO: Codex.

use crate::model::*;

#[derive(Clone, Debug)]
struct Lane {
    target: String,
    color: usize,
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

fn glyph(bits: u8) -> char {
    match bits {
        3 => '│',
        12 => '─',
        5 => '╯',
        10 => '╭',
        9 => '╰',
        6 => '╮',
        11 => '├',
        7 => '┤',
        13..=15 => '┼',
        _ => ' ',
    }
}

fn glyph_bits(ch: char) -> u8 {
    match ch {
        '│' => UP | DOWN,
        '─' => LEFT | RIGHT,
        '╯' => UP | LEFT,
        '╭' => DOWN | RIGHT,
        '╰' => UP | RIGHT,
        '╮' => DOWN | LEFT,
        '├' => UP | DOWN | RIGHT,
        '┤' => UP | DOWN | LEFT,
        '┼' => UP | DOWN | LEFT | RIGHT,
        _ => 0,
    }
}

fn overlay(cells: &mut [GraphCell], column: usize, bits: u8, color: usize) {
    if let Some(cell) = cells.get_mut(column) {
        cell.ch = glyph(glyph_bits(cell.ch) | bits);
        cell.color = color;
    }
}

fn put(cells: &mut [GraphCell], column: usize, ch: char, color: usize) {
    if let Some(cell) = cells.get_mut(column) {
        *cell = GraphCell { ch, color };
    }
}

fn horizontal(cells: &mut [GraphCell], from: usize, to: usize, color: usize) {
    let (left, right) = if from < to { (from, to) } else { (to, from) };
    for lane in left..right {
        overlay(cells, lane * 2 + 1, LEFT | RIGHT, color);
    }
    for lane in (left + 1)..right {
        overlay(cells, lane * 2, LEFT | RIGHT, color);
    }
}

fn shift_lane(cells: &mut [GraphCell], old: usize, new: usize, color: usize) {
    horizontal(cells, old, new, color);
    if new < old {
        overlay(cells, old * 2, UP | LEFT, color);
        overlay(cells, new * 2, DOWN | RIGHT, color);
    } else {
        overlay(cells, old * 2, UP | RIGHT, color);
        overlay(cells, new * 2, DOWN | LEFT, color);
    }
}

/// Una fila por commit, mismo orden que `commits` (orden topológico de `git::log`).
/// Commits cuyos padres no están en el slice (paginación) dejan su carril abierto hacia abajo.
pub fn build_graph(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Lane> = Vec::new();
    let mut next_color = 0;
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let matching: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(index, lane)| (lane.target == commit.id).then_some(index))
            .collect();
        let lane_index = if let Some(index) = matching.first() {
            *index
        } else {
            let index = lanes.len();
            lanes.push(Lane {
                target: commit.id.clone(),
                color: next_color,
            });
            next_color += 1;
            index
        };
        let before = lanes.clone();
        let node_color = lanes[lane_index].color;

        for &duplicate in matching.iter().skip(1).rev() {
            lanes.remove(duplicate);
        }

        if let Some(first_parent) = commit.parents.first() {
            lanes[lane_index].target.clone_from(first_parent);
        } else {
            lanes.remove(lane_index);
        }

        let mut parent_targets = Vec::new();
        let mut next_parent_position = (lane_index + 1).min(lanes.len());
        for parent in commit.parents.iter().skip(1) {
            let existed_before = before.iter().any(|lane| lane.target == *parent);
            let original_position =
                if let Some(position) = lanes.iter().position(|lane| lane.target == *parent) {
                    position
                } else {
                    let position = next_parent_position.min(lanes.len());
                    lanes.insert(
                        position,
                        Lane {
                            target: parent.clone(),
                            color: next_color,
                        },
                    );
                    next_color += 1;
                    next_parent_position = position + 1;
                    position
                };
            parent_targets.push((parent, original_position, existed_before));
        }

        let parent_positions: Vec<(usize, bool)> = parent_targets
            .into_iter()
            .map(|(parent, original_position, existed_before)| {
                let position = lanes
                    .iter()
                    .position(|lane| lane.target == *parent)
                    .unwrap_or(original_position);
                (position, existed_before)
            })
            .collect();

        let width = before.len().max(lanes.len()).max(lane_index + 1);
        let mut cells = vec![GraphCell { ch: ' ', color: 0 }; width * 2];

        for (old_index, old_lane) in before.iter().enumerate() {
            if old_index == lane_index || matching.contains(&old_index) {
                continue;
            }
            if let Some(new_index) = lanes.iter().position(|lane| lane.color == old_lane.color) {
                if new_index == old_index {
                    overlay(&mut cells, old_index * 2, UP | DOWN, old_lane.color);
                } else {
                    shift_lane(&mut cells, old_index, new_index, old_lane.color);
                }
            }
        }

        for &duplicate in matching.iter().skip(1) {
            horizontal(&mut cells, lane_index, duplicate, node_color);
            let toward_node = if duplicate > lane_index { LEFT } else { RIGHT };
            overlay(&mut cells, duplicate * 2, UP | toward_node, node_color);
        }

        for (position, existed_before) in parent_positions {
            horizontal(&mut cells, lane_index, position, node_color);
            let toward_node = if position > lane_index { LEFT } else { RIGHT };
            let vertical = if existed_before { UP | DOWN } else { DOWN };
            overlay(&mut cells, position * 2, vertical | toward_node, node_color);
        }

        put(
            &mut cells,
            lane_index * 2,
            if commit.parents.len() > 1 {
                '◉'
            } else {
                '●'
            },
            node_color,
        );
        rows.push(GraphRow {
            cells,
            lane: lane_index,
            color: node_color,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.to_owned(),
            parents: parents.iter().map(|value| (*value).to_owned()).collect(),
            ..Commit::default()
        }
    }

    fn text(row: &GraphRow) -> String {
        row.cells.iter().map(|cell| cell.ch).collect()
    }

    fn render(commits: &[Commit]) -> Vec<String> {
        build_graph(commits).iter().map(text).collect()
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = build_graph(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert_eq!(
            rows.iter().map(text).collect::<Vec<_>>(),
            ["● ", "● ", "● "]
        );
        assert!(rows.iter().all(|row| row.lane == 0 && row.color == 0));
    }

    #[test]
    fn branch_and_merge_open_and_converge() {
        let rows = build_graph(&[
            commit("m", &["a", "b"]),
            commit("a", &["base"]),
            commit("b", &["base"]),
            commit("base", &[]),
        ]);
        assert_eq!(rows[0].cells[0].ch, '◉');
        assert_eq!(rows[0].cells[1].ch, '─');
        assert_eq!(rows[0].cells[2].ch, '╮');
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[3].lane, 0);
        assert!(text(&rows[3]).contains('─'));
    }

    #[test]
    fn closing_branch_moves_right_lane_to_the_left() {
        let history = [
            commit("a", &["root-a"]),
            commit("b", &["root-b"]),
            commit("root-a", &[]),
            commit("root-b", &[]),
        ];
        assert_eq!(render(&history), ["● ", "│ ● ", "●─╯ ", "● "]);
    }

    #[test]
    fn merge_into_existing_lane_keeps_its_vertical() {
        let rows = build_graph(&[
            commit("tip", &["merge", "side"]),
            commit("merge", &["main", "side"]),
        ]);
        assert_eq!(text(&rows[1]), "◉─┤ ");
    }

    #[test]
    fn inserting_merge_parent_moves_existing_lane_right() {
        let rows = build_graph(&[
            commit("tip", &["merge", "side"]),
            commit("merge", &["main", "new-parent"]),
        ]);
        assert_eq!(text(&rows[1]), "◉─┼─╮ ");
    }

    #[test]
    fn renders_feature_merge_and_main_history() {
        let history = [
            commit("merge", &["main", "feature"]),
            commit("main", &["base"]),
            commit("feature", &["base"]),
            commit("base", &[]),
        ];
        assert_eq!(render(&history), ["◉─╮ ", "● │ ", "│ ● ", "●─╯ "]);
    }

    #[test]
    fn octopus_opens_every_parent() {
        let rows = build_graph(&[
            commit("m", &["a", "b", "c"]),
            commit("a", &[]),
            commit("b", &[]),
            commit("c", &[]),
        ]);
        assert_eq!(rows[0].cells.len(), 6);
        assert_eq!(rows[0].cells[0].ch, '◉');
        assert_eq!(rows[0].cells[2].ch, '┼');
        assert_eq!(rows[0].cells[4].ch, '╮');
    }

    #[test]
    fn missing_parent_keeps_lane_open() {
        let rows = build_graph(&[commit("a", &["outside"]), commit("other", &[])]);
        assert_eq!(rows[1].cells[0].ch, '│');
        assert_eq!(rows[1].cells[2].ch, '●');
    }

    #[test]
    fn handles_five_thousand_commits() {
        let commits: Vec<_> = (0..5000)
            .rev()
            .map(|n| {
                let id = n.to_string();
                if n == 0 {
                    commit(&id, &[])
                } else {
                    commit(&id, &[&(n - 1).to_string()])
                }
            })
            .collect();
        assert_eq!(build_graph(&commits).len(), 5000);
    }
}
