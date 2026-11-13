//! The flowchart model builder, equivalent to Mermaid's `FlowDB`. Port of Pele's
//! `src/diagrams/flowchart/db.ts`.
//!
//! Pele's `FlowDb` keeps nodes, classes, and tooltips in `Map`s. Here each is a `Vec` in insertion
//! order with a `BTreeMap` index for lookups, so iteration order never depends on hashing.

use std::collections::{BTreeMap, BTreeSet};

use super::shapes::is_valid_shape;
use super::types::{
    DocItem, FlowClassDef, FlowEdge, FlowNode, FlowSubgraph, FlowText, LabelType, LinkInfo,
};
use crate::error::{Code, Error, push_u32};
use crate::util::chars::is_space;
use crate::util::url;
use crate::yaml::{self, Value};

/// Options Pele's flowchart entry passes to `FlowDb`. Pele's `direction` and `warn` options are
/// never set by it, so they are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    /// Whether subgraphs without a `direction` inherit the diagram's.
    pub(crate) inherit_dir: bool,
    /// The most edges a diagram may have.
    pub(crate) max_edges: Option<usize>,
}

/// The parsed flowchart: Pele's `FlowDb`, which implements `FlowchartModel`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FlowDb {
    /// The accessible description.
    pub(crate) acc_descr: Option<String>,
    /// The accessible title.
    pub(crate) acc_title: Option<String>,
    /// Class definitions, in insertion order.
    pub(crate) classes: Vec<FlowClassDef>,
    /// Styles from `linkStyle default`.
    pub(crate) default_edge_style: Option<Vec<String>>,
    /// The curve from `linkStyle default interpolate`.
    pub(crate) default_interpolate: Option<String>,
    /// The diagram direction.
    pub(crate) direction: Option<String>,
    /// Edges, in insertion order.
    pub(crate) edges: Vec<FlowEdge>,
    /// The authored line of the statement being parsed, which the elements it names record.
    /// `None` unless the host asks for source lines.
    pub(crate) line: Option<u32>,
    /// Nodes, in insertion order.
    pub(crate) nodes: Vec<FlowNode>,
    /// Subgraphs, in the order they close.
    pub(crate) subgraphs: Vec<FlowSubgraph>,
    /// The front matter title.
    pub(crate) title: Option<String>,
    /// Tooltips by node id, in insertion order.
    pub(crate) tooltips: Vec<(String, String)>,

    class_index: BTreeMap<String, usize>,
    edge_by_id: BTreeMap<String, usize>,
    members: BTreeSet<String>,
    node_index: BTreeMap<String, usize>,
    options: Options,
    pair_count: BTreeMap<(String, String), usize>,
    sub_count: usize,
    subgraph_lookup: BTreeMap<String, usize>,
    tooltip_index: BTreeMap<String, usize>,
}

/// Pele's `labelType`: a known label type name, or Markdown.
fn label_type(r#type: Option<&str>) -> LabelType {
    match r#type {
        Some("string") => LabelType::String,
        Some("text") => LabelType::Text,
        _ => LabelType::Markdown,
    }
}

/// Adds `line`, the line of the statement being parsed, to an element's `lines`.
fn record(lines: &mut Vec<u32>, line: Option<u32>) {
    if let Some(line) = line {
        crate::lines::mark(lines, line);
    }
}

/// Strips one pair of surrounding double quotes.
fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text)
}

/// Pele's `destructStartLink`.
fn destruct_start_link(raw: &str) -> (&'static str, &'static str) {
    let mut s = js_trim(raw);
    let mut r#type = "arrow_open";
    match s.as_bytes().first() {
        Some(b'<') => r#type = "arrow_point",
        Some(b'x') => r#type = "arrow_cross",
        Some(b'o') => r#type = "arrow_circle",
        _ => {}
    }
    if r#type != "arrow_open" {
        s = s.get(1..).unwrap_or("");
    }

    let mut stroke = "normal";
    if s.contains('=') {
        stroke = "thick";
    }
    if s.contains('.') {
        stroke = "dotted";
    }
    (r#type, stroke)
}

/// Pele's `destructEndLink`. Link tokens are ASCII once trimmed, so byte slicing is safe.
fn destruct_end_link(raw: &str) -> LinkInfo {
    let s = js_trim(raw);
    let mut line = s.get(..s.len().saturating_sub(1)).unwrap_or("");
    let (r#type, double) = match s.as_bytes().last() {
        Some(b'x') => ("arrow_cross", s.starts_with('x')),
        Some(b'>') => ("arrow_point", s.starts_with('<')),
        Some(b'o') => ("arrow_circle", s.starts_with('o')),
        _ => ("arrow_open", false),
    };
    let r#type = if double {
        line = line.get(1..).unwrap_or("");
        match r#type {
            "arrow_cross" => "double_arrow_cross",
            "arrow_point" => "double_arrow_point",
            _ => "double_arrow_circle",
        }
    } else {
        r#type
    };

    let mut stroke = "normal";
    let mut length = i32::try_from(line.len()).unwrap_or(i32::MAX) - 1;
    if line.starts_with('=') {
        stroke = "thick";
    }
    if line.starts_with('~') {
        stroke = "invisible";
    }
    let dots = line.bytes().filter(|&b| b == b'.').count();
    if dots > 0 {
        stroke = "dotted";
        length = i32::try_from(dots).unwrap_or(i32::MAX);
    }

    LinkInfo {
        id: None,
        length: Some(length),
        stroke,
        text: None,
        r#type,
    }
}

impl FlowDb {
    /// Creates an empty model.
    pub(crate) fn new(options: Options) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    fn node_mut(&mut self, id: &str) -> Option<&mut FlowNode> {
        let index = *self.node_index.get(id)?;
        self.nodes.get_mut(index)
    }

    /// Adds or updates a node. `metadata` is the text of a `@{ ... }` block. Pele's `classes`
    /// and `dir` parameters are never passed by the parser, so they are left out.
    pub(crate) fn add_vertex(
        &mut self,
        id: &str,
        text: Option<FlowText>,
        r#type: Option<&str>,
        style: Option<Vec<String>>,
        props: Option<(String, String)>,
        metadata: Option<&str>,
    ) -> Result<(), Error> {
        if js_trim(id).is_empty() {
            return Ok(());
        }

        let mut doc = None;
        if let Some(metadata) = metadata {
            let mut source = String::with_capacity(metadata.len() + 4);
            if metadata.contains('\n') {
                source.push_str(metadata);
                source.push('\n');
            } else {
                source.push_str("{\n");
                source.push_str(metadata);
                source.push_str("\n}");
            }

            match yaml::parse(&source) {
                Ok(Value::Object(entries)) => doc = Some(entries),
                Ok(_) => {}
                Err(message) => {
                    let mut text = String::from("Metadata of \"");
                    text.push_str(id);
                    text.push_str("\" is not valid YAML. ");
                    text.push_str(&message);
                    return Err(flow_error(Code::Syntax, text));
                }
            }
        }

        let line = self.line;
        if let Some(&index) = self.subgraph_lookup.get(id)
            && let Some(doc) = &doc
        {
            if let Some(subgraph) = self.subgraphs.get_mut(index) {
                record(&mut subgraph.lines, line);
                let merged = subgraph.metadata.get_or_insert_with(Vec::new);
                for (key, value) in doc {
                    yaml::set(merged, key.clone(), value.clone());
                }
            }
            return Ok(());
        }

        if let Some(&index) = self.edge_by_id.get(id) {
            if let Some(edge) = self.edges.get_mut(index) {
                record(&mut edge.lines, line);
            }
            if let (Some(edge), Some(doc)) = (self.edges.get_mut(index), &doc) {
                if let Some(value) = get(doc, "animate") {
                    edge.animate = Some(value.clone());
                }
                if let Some(value) = get(doc, "animation") {
                    edge.animation = Some(value.clone());
                }
                if let Some(value) = get(doc, "curve") {
                    edge.interpolate = Some(value.clone());
                }
            }
            return Ok(());
        }

        if !self.node_index.contains_key(id) {
            self.node_index.insert(id.to_owned(), self.nodes.len());
            self.nodes.push(FlowNode {
                asset_height: None,
                asset_width: None,
                classes: Vec::new(),
                constraint: None,
                dir: None,
                form: None,
                icon: None,
                id: id.to_owned(),
                img: None,
                label_type: LabelType::Text,
                lines: Vec::new(),
                link: None,
                link_target: None,
                pos: None,
                props: None,
                styles: Vec::new(),
                text: None,
                r#type: None,
            });
        }
        let Some(vertex) = self.node_mut(id) else {
            return Ok(());
        };
        record(&mut vertex.lines, line);

        if let Some(text) = text {
            vertex.label_type = text.r#type;
            vertex.text = Some(unquote(js_trim(&text.text)).to_owned());
        } else if vertex.text.is_none() {
            vertex.text = Some(id.to_owned());
        }
        if let Some(r#type) = r#type {
            vertex.r#type = Some(r#type.to_owned());
        }
        if let Some(style) = style {
            vertex.styles.extend(style);
        }
        match &mut vertex.props {
            None => vertex.props = Some(props.into_iter().collect()),
            Some(existing) => {
                // Without an own `__proto__`, `Object.assign` sets the prototype instead.
                if let Some((key, value)) = props {
                    match existing.iter_mut().find(|(own, _)| *own == key) {
                        Some(entry) => entry.1 = value,
                        None if key == "__proto__" => {}
                        None => existing.push((key, value)),
                    }
                }
            }
        }

        let Some(doc) = doc else {
            return Ok(());
        };

        if let Some(shape) = get(&doc, "shape").filter(|value| value.truthy()) {
            let shape = js_string(shape);
            if has_upper(&shape) || shape.contains('_') {
                let mut text = String::from("No such shape: ");
                text.push_str(&shape);
                text.push_str(". Shape names should be lowercase.");
                return Err(flow_error(Code::Semantic, text));
            }
            if !is_valid_shape(&shape) {
                let mut text = String::from("No such shape: ");
                text.push_str(&shape);
                text.push('.');
                return Err(flow_error(Code::Semantic, text));
            }
            vertex.r#type = Some(shape);
        }

        let label = match get(&doc, "label") {
            Some(Value::String(label)) => Some(label.as_str()),
            _ => None,
        };
        let blank_label = label.is_none_or(|label| js_trim(label).is_empty());
        if let Some(value) = get(&doc, "label").filter(|value| value.truthy()) {
            vertex.text = Some(js_string(value));
            vertex.label_type = label_type(match get(&doc, "labelType") {
                Some(Value::String(name)) => Some(name.as_str()),
                _ => None,
            });
        }
        if let Some(value) = get(&doc, "icon").filter(|value| value.truthy()) {
            vertex.icon = Some(js_string(value));
            if blank_label && vertex.text.as_deref() == Some(id) {
                vertex.text = Some(String::new());
            }
        }
        if let Some(value) = get(&doc, "form").filter(|value| value.truthy()) {
            vertex.form = Some(js_string(value));
        }
        if let Some(value) = get(&doc, "pos").filter(|value| value.truthy()) {
            vertex.pos = Some(js_string(value));
        }
        if let Some(value) = get(&doc, "img").filter(|value| value.truthy()) {
            vertex.img = Some(js_string(value));
            if blank_label && vertex.text.as_deref() == Some(id) {
                vertex.text = Some(String::new());
            }
        }
        if let Some(value) = get(&doc, "constraint").filter(|value| value.truthy()) {
            vertex.constraint = Some(js_string(value));
        }
        if let Some(value) = get(&doc, "w").filter(|value| value.truthy()) {
            vertex.asset_width = Some(js_number(value));
        }
        if let Some(value) = get(&doc, "h").filter(|value| value.truthy()) {
            vertex.asset_height = Some(js_number(value));
        }
        Ok(())
    }

    /// Adds one edge. `id` is the user-defined id, without its `@`.
    fn add_single_link(
        &mut self,
        start: &str,
        end: &str,
        info: &LinkInfo,
        id: Option<&str>,
    ) -> Result<(), Error> {
        let mut edge = FlowEdge {
            animate: None,
            animation: None,
            classes: Vec::new(),
            end: end.to_owned(),
            id: None,
            interpolate: self.default_interpolate.clone().map(Value::String),
            is_user_defined_id: false,
            label_type: LabelType::Text,
            length: None,
            lines: self.line.into_iter().collect(),
            start: start.to_owned(),
            stroke: Some(info.stroke.to_owned()),
            style: None,
            text: String::new(),
            r#type: Some(info.r#type.to_owned()),
        };
        if let Some(text) = &info.text {
            unquote(js_trim(&text.text)).clone_into(&mut edge.text);
            edge.label_type = label_type(Some(text.r#type.as_str()));
        }
        edge.length = info.length.map(|length| length.min(10));

        let pair = (start.to_owned(), end.to_owned());
        let existing = self.pair_count.get(&pair).copied().unwrap_or(0);
        let edge_id = match id {
            Some(id) if !id.is_empty() && !self.edge_by_id.contains_key(id) => {
                edge.is_user_defined_id = true;
                id.to_owned()
            }
            _ => {
                let mut generated = String::from("L_");
                generated.push_str(start);
                generated.push('_');
                generated.push_str(end);
                generated.push('_');
                let count = if existing == 0 { 0 } else { existing + 1 };
                push_u32(&mut generated, u32::try_from(count).unwrap_or(u32::MAX));
                generated
            }
        };

        if let Some(max) = self.options.max_edges
            && self.edges.len() >= max
        {
            let mut text = String::from("Edge limit exceeded. ");
            push_u32(
                &mut text,
                u32::try_from(self.edges.len()).unwrap_or(u32::MAX),
            );
            text.push_str(" edges found, but the limit is ");
            push_u32(&mut text, u32::try_from(max).unwrap_or(u32::MAX));
            text.push('.');
            return Err(flow_error(Code::Limit, text));
        }

        if !self.edge_by_id.contains_key(&edge_id) {
            self.edge_by_id.insert(edge_id.clone(), self.edges.len());
        }
        edge.id = Some(edge_id);
        self.edges.push(edge);
        *self.pair_count.entry(pair).or_insert(0) += 1;
        Ok(())
    }

    /// Adds an edge from every start to every end. Only the edge from the last start to the
    /// first end takes the user-defined id.
    pub(crate) fn add_link(
        &mut self,
        starts: &[String],
        ends: &[String],
        info: &LinkInfo,
    ) -> Result<(), Error> {
        let id = info.id.as_deref().map(|id| id.replacen('@', "", 1));
        let last_start = starts.last();
        let first_end = ends.first();
        for start in starts {
            for end in ends {
                let own = Some(start) == last_start && Some(end) == first_end;
                self.add_single_link(start, end, info, if own { id.as_deref() } else { None })?;
            }
        }
        Ok(())
    }

    /// Sets the curve of the edges at `positions`, or the default curve.
    pub(crate) fn update_link_interpolate(
        &mut self,
        positions: &[String],
        interpolate: &str,
    ) -> Result<(), Error> {
        for pos in positions {
            if pos == "default" {
                self.default_interpolate = Some(interpolate.to_owned());
            } else {
                let line = self.line;
                let edge = self.edge_at(pos)?;
                edge.interpolate = Some(Value::String(interpolate.to_owned()));
                record(&mut edge.lines, line);
            }
        }
        Ok(())
    }

    /// Sets the styles of the edges at `positions`, or the default styles.
    pub(crate) fn update_link(
        &mut self,
        positions: &[String],
        mut style: Vec<String>,
    ) -> Result<(), Error> {
        // Pele assigns one array to every edge and appends `fill:none` to it at the first edge,
        // so every edge of the statement sees the appended value.
        if positions.iter().any(|pos| pos != "default")
            && !style.is_empty()
            && !style.iter().any(|s| s.starts_with("fill"))
        {
            style.push("fill:none".to_owned());
        }
        for pos in positions {
            if pos == "default" {
                self.default_edge_style = Some(style.clone());
            } else {
                let line = self.line;
                let edge = self.edge_at(pos)?;
                edge.style = Some(style.clone());
                record(&mut edge.lines, line);
            }
        }
        Ok(())
    }

    fn edge_at(&mut self, pos: &str) -> Result<&mut FlowEdge, Error> {
        let count = self.edges.len();
        let index = pos.parse::<usize>().ok().filter(|&index| index < count);
        if let Some(edge) = index.and_then(|index| self.edges.get_mut(index)) {
            return Ok(edge);
        }

        let mut text = String::from("The index ");
        text.push_str(pos);
        text.push_str(
            " for linkStyle is out of bounds. Valid indices for linkStyle are between 0 and ",
        );
        if count == 0 {
            text.push_str("-1");
        } else {
            push_u32(&mut text, u32::try_from(count - 1).unwrap_or(u32::MAX));
        }
        text.push_str(". (Help: Ensure that the index is within the range of existing edges.)");
        Err(flow_error(Code::Semantic, text))
    }

    /// Defines classes from `classDef`.
    pub(crate) fn add_class(&mut self, ids: &str, styles: &[String]) {
        let joined = styles.join(",");
        let escaped = replace_all(&joined, "\\,", "§§§");
        let separated = replace_all(&escaped, ",", ";");
        let restored = replace_all(&separated, "§§§", ",");

        for id in ids.split(',') {
            let index = if let Some(&index) = self.class_index.get(id) {
                index
            } else {
                self.class_index.insert(id.to_owned(), self.classes.len());
                self.classes.push(FlowClassDef {
                    id: id.to_owned(),
                    styles: Vec::new(),
                    text_styles: Vec::new(),
                });
                self.classes.len() - 1
            };
            let Some(class) = self.classes.get_mut(index) else {
                continue;
            };
            for s in restored.split(';') {
                if s.contains("color") {
                    class.text_styles.push(s.replacen("fill", "bgFill", 1));
                }
                class.styles.push(s.to_owned());
            }
        }
    }

    /// Sets the diagram direction, mapping arrow symbols to names.
    pub(crate) fn set_direction(&mut self, dir: &str) {
        let mut d = js_trim(dir);
        if d.contains('<') {
            d = "RL";
        }
        if d.contains('^') {
            d = "BT";
        }
        if d.contains('>') {
            d = "LR";
        }
        if d.contains('v') {
            d = "TB";
        }
        if d == "TD" {
            d = "TB";
        }
        self.direction = Some(d.to_owned());
    }

    /// Adds a class to the nodes, edges, and subgraphs named in a comma-separated list.
    pub(crate) fn set_class(&mut self, ids: &str, class_name: &str) {
        let line = self.line;
        for id in ids.split(',') {
            if let Some(node) = self.node_mut(id) {
                node.classes.push(class_name.to_owned());
                record(&mut node.lines, line);
            }
            if let Some(edge) = self.edge_by_id.get(id).and_then(|&i| self.edges.get_mut(i)) {
                edge.classes.push(class_name.to_owned());
                record(&mut edge.lines, line);
            }
            if let Some(subgraph) = self
                .subgraph_lookup
                .get(id)
                .and_then(|&i| self.subgraphs.get_mut(i))
            {
                subgraph.classes.push(class_name.to_owned());
                record(&mut subgraph.lines, line);
            }
        }
    }

    /// Sets the tooltip of each node in a comma-separated list.
    pub(crate) fn set_tooltip(&mut self, ids: &str, tooltip: &str) {
        for id in ids.split(',') {
            if let Some(&index) = self.tooltip_index.get(id) {
                if let Some(entry) = self.tooltips.get_mut(index) {
                    tooltip.clone_into(&mut entry.1);
                }
            } else {
                self.tooltip_index
                    .insert(id.to_owned(), self.tooltips.len());
                self.tooltips.push((id.to_owned(), tooltip.to_owned()));
            }
        }
    }

    /// Sets the click target of each node in a comma-separated list and marks them clickable.
    pub(crate) fn set_link(&mut self, ids: &str, link: &str, target: Option<&str>) {
        let url = js_trim(link);
        for id in ids.split(',') {
            if let Some(vertex) = self.node_mut(id) {
                vertex.link = (!url.is_empty()).then(|| url::safe_url(url));
                vertex.link_target = target.map(str::to_owned);
            }
        }
        self.set_class(ids, "clickable");
    }

    /// Marks nodes clickable. Callbacks are never executed, as in Mermaid's strict mode.
    pub(crate) fn set_click_event(&mut self, ids: &str) {
        self.set_class(ids, "clickable");
    }

    /// Sets the accessible title without leading whitespace.
    pub(crate) fn set_acc_title(&mut self, text: &str) {
        self.acc_title = Some(
            text.trim_start_matches(|c| is_space(u32::from(c)))
                .to_owned(),
        );
    }

    /// Sets the accessible description, removing indentation after line breaks.
    pub(crate) fn set_acc_description(&mut self, text: &str) {
        // `text.replace(/\n\s+/g, '\n')`.
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if c == '\n' {
                while chars.next_if(|&next| is_space(u32::from(next))).is_some() {}
            }
        }
        self.acc_descr = Some(out);
    }

    /// Adds a subgraph from the items its body collected and returns its id. `same` is set when
    /// the title is the id text itself, as Pele compares the two objects.
    pub(crate) fn add_subgraph(
        &mut self,
        id_text: Option<&FlowText>,
        list: &[DocItem],
        title_text: Option<&FlowText>,
        same: bool,
    ) -> String {
        let mut title = title_text.map_or("", |title| title.text.as_str());
        let mut id = id_text.map(|id| js_trim(&id.text).to_owned());
        if id_text.is_some() && same && title.chars().any(|c| is_space(u32::from(c))) {
            id = None;
        }

        let mut seen = BTreeSet::new();
        let mut node_list: Vec<String> = Vec::new();
        let mut dir = None;
        let mut add = |item: &String| {
            if !js_trim(item).is_empty() && seen.insert(item.clone()) {
                node_list.push(item.clone());
            }
        };
        for entry in list {
            match entry {
                DocItem::Str(item) => add(item),
                DocItem::Ids(items) => items.iter().for_each(&mut add),
                DocItem::Dir(value) => dir = Some((*value).to_owned()),
            }
        }
        if dir.is_none() && self.options.inherit_dir {
            dir.clone_from(&self.direction);
        }

        let id = id.unwrap_or_else(|| {
            let mut generated = String::from("subGraph");
            push_u32(
                &mut generated,
                u32::try_from(self.sub_count).unwrap_or(u32::MAX),
            );
            generated
        });
        if title.is_empty() {
            title = "";
        }
        self.sub_count += 1;

        // A node belongs to the first subgraph that lists it, and a subgraph never contains itself.
        let nodes: Vec<String> = node_list
            .into_iter()
            .filter(|node| !self.members.contains(node) && *node != id)
            .collect();
        for node in &nodes {
            self.members.insert(node.clone());
        }

        if let Some(&index) = self.subgraph_lookup.get(&id) {
            if let Some(existing) = self.subgraphs.get_mut(index) {
                existing.nodes.extend(nodes);
                record(&mut existing.lines, self.line);
            }
        } else {
            self.subgraph_lookup
                .insert(id.clone(), self.subgraphs.len());
            self.subgraphs.push(FlowSubgraph {
                classes: Vec::new(),
                dir,
                id: id.clone(),
                label_type: label_type(title_text.map(|title| title.r#type.as_str())),
                lines: self.line.into_iter().collect(),
                metadata: None,
                nodes,
                title: js_trim(title).to_owned(),
            });
        }
        id
    }

    /// Pele's `destructLink`: reads an arrow, and its opening half when the label sits inside it.
    pub(crate) fn destruct_link(end: &str, start: Option<&str>) -> LinkInfo {
        let info = destruct_end_link(end);
        let Some(start) = start.filter(|start| !start.is_empty()) else {
            return info;
        };

        let invalid = LinkInfo {
            id: None,
            length: None,
            stroke: "INVALID",
            text: None,
            r#type: "INVALID",
        };
        let (start_type, stroke) = destruct_start_link(start);
        if stroke != info.stroke {
            return invalid;
        }
        let r#type = if start_type == "arrow_open" {
            info.r#type
        } else if start_type != info.r#type {
            return invalid;
        } else {
            match start_type {
                "arrow_point" => "double_arrow_point",
                "arrow_cross" => "double_arrow_cross",
                _ => "double_arrow_circle",
            }
        };

        LinkInfo {
            id: None,
            length: info.length,
            stroke,
            text: None,
            r#type,
        }
    }

    /// The subgraph with `id`.
    pub(crate) fn subgraph(&self, id: &str) -> Option<&FlowSubgraph> {
        self.subgraph_lookup
            .get(id)
            .and_then(|&index| self.subgraphs.get(index))
    }
}

/// A flowchart error without a source position.
#[cold]
fn flow_error(code: Code, message: String) -> Error {
    let mut error = Error::new(code, message);
    error.r#type = Some("flowchart");
    error
}

/// `str.replace(from, to)` with a global pattern, without `str::replace`.
fn replace_all(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        out.push_str(rest.get(..at).unwrap_or(""));
        out.push_str(to);
        rest = rest.get(at + from.len()..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// Whether lowercasing `text` would change it, as `text !== text.toLowerCase()` tests.
fn has_upper(text: &str) -> bool {
    text.chars().any(|c| {
        let mut lower = c.to_lowercase();
        lower.next() != Some(c) || lower.next().is_some()
    })
}

/// JavaScript's `String.prototype.trim`, which strips what `\s` matches.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(|c| is_space(u32::from(c)))
}

/// A property of a YAML mapping.
fn get<'v>(doc: &'v [(String, Value)], key: &str) -> Option<&'v Value> {
    doc.iter()
        .find(|(own, _)| own == key)
        .map(|(_, value)| value)
}

/// JavaScript's `String(value)` for a YAML value.
fn js_string(value: &Value) -> String {
    let mut out = String::new();
    value.push_js_string(&mut out);
    out
}

/// JavaScript's `Number(value)` for a YAML value.
fn js_number(value: &Value) -> f64 {
    match value {
        Value::Array(_) => string_to_number(&js_string(value)),
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::Null => 0.0,
        Value::Number(value) => *value,
        Value::Object(_) => f64::NAN,
        Value::String(text) => string_to_number(text),
    }
}

/// JavaScript's `StringToNumber`.
fn string_to_number(text: &str) -> f64 {
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }

    let bytes = text.as_bytes();
    if let (Some(b'0'), Some(&prefix)) = (bytes.first(), bytes.get(1)) {
        let radix = match prefix {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            let digits = text.get(2..).unwrap_or("");
            if digits.is_empty() {
                return f64::NAN;
            }
            let mut value = 0.0;
            for c in digits.chars() {
                let Some(digit) = c.to_digit(radix) else {
                    return f64::NAN;
                };
                value = value * f64::from(radix) + f64::from(digit);
            }
            return value;
        }
    }

    let unsigned = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    if unsigned == "Infinity" {
        return if text.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }

    // StrDecimalLiteral: digits with an optional fraction, then an optional exponent.
    let b = unsigned.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
        digits += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    crate::util::float::parse(text).unwrap_or(f64::NAN)
}
