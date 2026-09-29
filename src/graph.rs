//! Note graph derived from the same resolver as editor navigation.
use crate::index::{Index, Resolution};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub label: String,
    pub missing: bool,
    pub position: [f32; 2],
    pub color: Option<u32>,
}
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize)>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ColorGroup {
    pub query: String,
    pub color: u32,
}
impl Default for ColorGroup {
    fn default() -> Self {
        Self {
            query: String::new(),
            color: 0x9775fa,
        }
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    #[serde(skip)]
    pub root: Option<PathBuf>,
    pub depth: usize,
    pub query: String,
    pub orphans: bool,
    pub missing: bool,
    pub controls_open: bool,
    pub node_size: f32,
    pub link_width: f32,
    pub text_scale: f32,
    pub arrows: bool,
    pub labels: bool,
    pub groups: Vec<ColorGroup>,
    pub center_force: f32,
    pub repel_force: f32,
    pub link_force: f32,
    pub link_distance: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            root: None,
            depth: 1,
            query: String::new(),
            orphans: true,
            missing: false,
            controls_open: true,
            node_size: 1.,
            link_width: 1.,
            text_scale: 1.,
            arrows: false,
            labels: true,
            groups: vec![],
            center_force: 1.,
            repel_force: 1.,
            link_force: 1.,
            link_distance: 1.,
        }
    }
}
impl Options {
    pub fn normalize(&mut self) {
        self.depth = self.depth.clamp(1, 5);
        for group in &mut self.groups {
            group.color &= 0xffffff;
        }
        for (value, min, max) in [
            (&mut self.node_size, 0.5, 3.),
            (&mut self.link_width, 0.25, 4.),
            (&mut self.text_scale, 0.5, 2.),
            (&mut self.center_force, 0., 5.),
            (&mut self.repel_force, 0., 5.),
            (&mut self.link_force, 0., 5.),
            (&mut self.link_distance, 0.2, 3.),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                1.
            };
        }
    }
}
impl Graph {
    pub fn build(index: &Index, options: &Options) -> Self {
        let mut normalized_options = options.clone();
        normalized_options.normalize();
        let options = &normalized_options;
        fn parse_query(text: &str) -> Result<crate::search::Query, String> {
            let text = text.trim();
            if text.starts_with('#') && !text.contains(char::is_whitespace) {
                crate::search::Query::parse(&format!("tag:{text}"))
            } else {
                crate::search::Query::parse(text)
            }
        }
        let query = match parse_query(&options.query) {
            Ok(query) => query,
            Err(error) => {
                return Self {
                    error: Some(error),
                    ..Default::default()
                };
            }
        };
        let matches = |query: &crate::search::Query, path: &Path| {
            let note = index.notes.get(path);
            let text = format!(
                "{}\n{}",
                path.to_string_lossy(),
                note.map_or("", |n| n.text.as_str())
            );
            query.matches(path, &text, note.map_or(&[], |n| n.parsed.tags.as_slice()))
        };
        let mut error = None;
        let groups: Vec<_> = options
            .groups
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                if g.query.trim().is_empty() {
                    return None;
                }
                match parse_query(&g.query) {
                    Ok(q) => Some((q, g.color)),
                    Err(e) => {
                        error = Some(format!("颜色分组 {}：{e}", i + 1));
                        None
                    }
                }
            })
            .collect();
        let mut paths: BTreeMap<PathBuf, bool> =
            index.notes.keys().map(|p| (p.clone(), false)).collect();
        let mut links = BTreeSet::new();
        for (from, note) in &index.notes {
            let targets = note
                .parsed
                .links
                .iter()
                .map(|l| {
                    let resolution = index.resolve(from, &l.target);
                    if matches!(resolution, Resolution::Missing(_))
                        && crate::rendering::attachment_target(&l.target)
                    {
                        Resolution::Invalid
                    } else {
                        resolution
                    }
                })
                .chain(
                    note.parsed
                        .standard_links
                        .iter()
                        .map(|(url, _)| index.resolve_markdown(from, url).0),
                );
            for target in targets {
                let target = match target {
                    Resolution::Found(p) => p,
                    Resolution::Missing(p) if options.missing => {
                        paths.entry(p.clone()).or_insert(true);
                        p
                    }
                    _ => continue,
                };
                if target != *from {
                    links.insert((from.clone(), target));
                }
            }
        }
        let mut neighbors: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
        for (a, b) in &links {
            neighbors.entry(a.clone()).or_default().insert(b.clone());
            neighbors.entry(b.clone()).or_default().insert(a.clone());
        }
        let local = options.root.as_ref().map(|root| {
            let mut seen = BTreeSet::from([root.clone()]);
            let mut queue = VecDeque::from([(root.clone(), 0)]);
            while let Some((path, depth)) = queue.pop_front() {
                if depth >= options.depth {
                    continue;
                }
                for next in neighbors.get(&path).into_iter().flatten() {
                    if seen.insert(next.clone()) {
                        queue.push_back((next.clone(), depth + 1));
                    }
                }
            }
            seen
        });
        paths.retain(|p, _| {
            (options.query.trim().is_empty() || matches(&query, p))
                && local.as_ref().is_none_or(|set| set.contains(p))
                && (options.orphans
                    || neighbors.contains_key(p)
                    || options.root.as_ref() == Some(p))
        });
        let ids: BTreeMap<_, _> = paths
            .keys()
            .cloned()
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect();
        let nodes = paths
            .into_iter()
            .map(|(path, missing)| Node {
                color: groups
                    .iter()
                    .find(|(q, _)| matches(q, &path))
                    .map(|(_, color)| *color),
                label: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path,
                missing,
                position: [0., 0.],
            })
            .collect();
        let edges = links
            .into_iter()
            .filter_map(|(a, b)| Some((*ids.get(&a)?, *ids.get(&b)?)))
            .collect();
        let mut graph = Self {
            nodes,
            edges,
            error,
        };
        graph.layout(options);
        graph
    }
    fn layout(&mut self, options: &Options) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let angle = i as f32 * 2.3999631;
            let radius = 45. * (i as f32).sqrt();
            node.position = [angle.cos() * radius, angle.sin() * radius];
        }
        // Aggregate nearby cells: bounded work per node even for large vaults.
        for _ in 0..100 {
            let mut grid: BTreeMap<(i32, i32), (usize, [f32; 2])> = BTreeMap::new();
            for node in &self.nodes {
                let cell = (
                    (node.position[0] / 100.).floor() as i32,
                    (node.position[1] / 100.).floor() as i32,
                );
                let entry = grid.entry(cell).or_default();
                entry.0 += 1;
                entry.1[0] += node.position[0];
                entry.1[1] += node.position[1];
            }
            let mut force = vec![[0f32; 2]; n];
            for (i, node) in self.nodes.iter().enumerate() {
                let [x, y] = node.position;
                let (gx, gy) = ((x / 100.).floor() as i32, (y / 100.).floor() as i32);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        if let Some(&(mut count, mut sum)) = grid.get(&(gx + dx, gy + dy)) {
                            if dx == 0 && dy == 0 {
                                count -= 1;
                                sum[0] -= x;
                                sum[1] -= y;
                            }
                            if count == 0 {
                                continue;
                            }
                            let vx = x - sum[0] / count as f32;
                            let vy = y - sum[1] / count as f32;
                            let square = (vx * vx + vy * vy).max(16.);
                            let repulsion = 1200. * options.repel_force * count as f32 / square;
                            force[i][0] += vx * repulsion;
                            force[i][1] += vy * repulsion;
                        }
                    }
                }
                force[i][0] -= x * 0.015 * options.center_force;
                force[i][1] -= y * 0.015 * options.center_force;
            }
            for &(a, b) in &self.edges {
                let dx = self.nodes[b].position[0] - self.nodes[a].position[0];
                let dy = self.nodes[b].position[1] - self.nodes[a].position[1];
                let distance = (dx * dx + dy * dy).sqrt().max(1.);
                let strength =
                    (distance - 90. * options.link_distance) * 0.025 * options.link_force
                        / distance;
                force[a][0] += dx * strength;
                force[a][1] += dy * strength;
                force[b][0] -= dx * strength;
                force[b][1] -= dy * strength;
            }
            for (node, force) in self.nodes.iter_mut().zip(force) {
                node.position[0] += force[0].clamp(-15., 15.) * 0.3;
                node.position[1] += force[1].clamp(-15., 15.) * 0.3;
            }
        }
    }
    pub fn find(&self, path: &Path) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn force_controls_change_layout_and_invalid_values_stay_finite() {
        let mut index = Index::default();
        index.update("a.md".into(), "[[b]]".into());
        index.update("b.md".into(), "".into());
        let mut options = Options {
            repel_force: 0.,
            center_force: 0.,
            link_force: 5.,
            link_distance: 0.2,
            ..Default::default()
        };
        let distance = |g: &Graph| {
            let a = g.nodes[0].position;
            let b = g.nodes[1].position;
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let close = Graph::build(&index, &options);
        options.link_distance = 3.;
        let far = Graph::build(&index, &options);
        assert!(distance(&far) > distance(&close) + 50.);
        options.center_force = f32::NAN;
        options.repel_force = f32::INFINITY;
        options.link_distance = -20.;
        let graph = Graph::build(&index, &options);
        assert!(
            graph
                .nodes
                .iter()
                .all(|n| n.position.iter().all(|x| x.is_finite()))
        );
    }
    #[test]
    fn graph_filters_and_color_groups_share_search_rules_and_order() {
        let mut index = Index::default();
        index.update("project/a.md".into(), "---\ntags: [work]\n---\nText".into());
        index.update("archive/b.md".into(), "#work".into());
        index.update("other.md".into(), "#personal".into());
        let mut options = Options {
            query: "tag:work -path:archive".into(),
            groups: vec![
                ColorGroup {
                    query: "path:project".into(),
                    color: 0xff0000,
                },
                ColorGroup {
                    query: "tag:work".into(),
                    color: 0x0000ff,
                },
            ],
            ..Default::default()
        };
        let graph = Graph::build(&index, &options);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].color, Some(0xff0000));
        options.groups.swap(0, 1);
        let graph = Graph::build(&index, &options);
        assert_eq!(graph.nodes[0].color, Some(0x0000ff));
        options.query = "#work".into();
        assert_eq!(Graph::build(&index, &options).nodes.len(), 2);
        options.groups[0].query = "tag:".into();
        let graph = Graph::build(&index, &options);
        assert!(graph.error.is_some());
        assert_eq!(graph.nodes.len(), 2);
        options.query = "path:".into();
        let graph = Graph::build(&index, &options);
        assert!(graph.error.is_some());
        assert!(graph.nodes.is_empty());
    }
    #[test]
    fn graph_uses_resolved_links_and_local_depth() {
        let mut index = Index::default();
        index.update(
            "a.md".into(),
            "[[b]] [also](b.md) [[missing]] ![[picture.svg]] `[[fake]]`".into(),
        );
        index.update("b.md".into(), "[[c]]".into());
        index.update("c.md".into(), "#tag".into());
        index.update("orphan.md".into(), "".into());
        let graph = Graph::build(&index, &Options::default());
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 2);
        let graph = Graph::build(
            &index,
            &Options {
                root: Some("a.md".into()),
                missing: true,
                ..Default::default()
            },
        );
        assert_eq!(graph.nodes.len(), 3);
        assert!(graph.find(Path::new("c.md")).is_none());
        assert!(
            graph
                .nodes
                .iter()
                .all(|n| n.position.iter().all(|v| v.is_finite()))
        );
        let graph = Graph::build(
            &index,
            &Options {
                orphans: false,
                ..Default::default()
            },
        );
        assert_eq!(graph.nodes.len(), 3);
    }
}
