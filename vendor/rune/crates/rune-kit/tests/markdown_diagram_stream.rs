//! A ```` ```mermaid ```` fence streaming into [`Markdown`]: drawn from its complete lines,
//! never shown as source first, and still when its fence closes.

use rune::runtime::ElementInfo;
use rune::{Cx, El, Env, Runtime, col};
use rune_kit::markdown::{Block, parse};
use rune_kit::{Markdown, MarkdownStyle};

type Doc = (String, bool);

fn view(s: &Doc, _: &mut Cx<Doc>) -> El {
    col().w_full().child(Markdown::new(s.0.clone()).streaming(s.1).render(&MarkdownStyle::neutral(true)))
}

fn keys(rt: &Runtime<Doc>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.key).collect()
}

/// The diagram's nodes, edges and frames, and their labels.
fn pieces(rt: &Runtime<Doc>) -> Vec<ElementInfo> {
    let mut v: Vec<ElementInfo> = rt.elements().into_iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("n:") || k.starts_with("e:") || k.starts_with("g:"))).collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

fn settle(rt: &mut Runtime<Doc>) {
    for _ in 0..120 {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn an_open_mermaid_fence_is_a_diagram_drawn_from_its_complete_lines() {
    assert!(matches!(&parse("```mermaid\ngraph TD\nA-->B")[0], Block::Mermaid(_)));
    let mut rt = Runtime::new(("```".to_owned(), true), view, (600.0, 800.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for more in ["mer", "maid\n", "graph T", "D\n  A[Start] --> B[Ch", "eck]\n"] {
        let b0 = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("b0"));
        assert_eq!(b0.map_or(0.0, |b| b.rect[3]), 0.0, "no flash of source or an empty frame before {more:?}: {:?}", keys(&rt));
        rt.update(|s, _| s.0.push_str(more));
        rt.frame(1.0 / 60.0);
    }
    let k = keys(&rt);
    assert!(k.iter().any(|k| k == "n:A") && k.iter().any(|k| k == "n:B") && !k.iter().any(|k| k == "source"), "{k:?}");
    // The line still arriving draws nothing yet.
    rt.update(|s, _| s.0.push_str("  B --> C[Do"));
    rt.frame(1.0 / 60.0);
    assert!(!keys(&rt).iter().any(|k| k == "n:C"));
    rt.update(|s, _| s.0.push_str("ne]\n"));
    rt.frame(1.0 / 60.0);
    assert!(keys(&rt).iter().any(|k| k == "n:C"));

    // A type the subset doesn't draw streams as its source.
    let mut rt = Runtime::new(("```mermaid\nkanban\n  Todo\n".to_owned(), true), view, (600.0, 800.0), Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(keys(&rt).iter().any(|k| k == "source"));
}

#[test]
fn closing_the_fence_moves_nothing() {
    let body = "```mermaid\ngraph LR\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Ship]\n  B -->|no| D[Fix]\n  D --> B\n";
    for still_streaming in [true, false] {
        let mut rt = Runtime::new((body.to_owned(), true), view, (600.0, 800.0), Env::new(true, 1.0));
        rt.frame(0.0);
        settle(&mut rt);
        let before = pieces(&rt);
        assert!(before.len() > 8, "{:?}", before.iter().map(|e| e.key.clone()).collect::<Vec<_>>());
        rt.update(|s, _| {
            s.0.push_str("```\n");
            s.1 = still_streaming;
        });
        rt.frame(1.0 / 60.0);
        let after = pieces(&rt);
        assert_eq!(before.len(), after.len());
        for (b, a) in before.iter().zip(&after) {
            assert_eq!((&b.path, b.rect, b.opacity), (&a.path, a.rect, a.opacity), "{:?} moved when the fence closed", b.key);
            assert!(!a.moving, "{:?} animates", a.key);
        }
    }
}

#[test]
fn a_finished_diagram_shows_whole_the_frame_it_mounts() {
    let doc = "Before\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n  B-->>A: hello\n```\n";
    let mut rt = Runtime::new((String::new(), false), view, (600.0, 800.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.update(|s, _| s.0 = doc.to_owned());
    rt.frame(1.0 / 60.0);
    let shown = pieces(&rt);
    assert!(shown.len() > 4, "{:?}", keys(&rt));
    for e in &shown {
        assert_eq!((e.opacity, e.scale), (1.0, 1.0), "{:?} fades or grows in", e.key);
    }
}
