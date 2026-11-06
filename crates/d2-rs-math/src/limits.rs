// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go tex/limits.go: node-specific coreMO traversal,
// setLimits family replacement, and parseLimits compact-script origins.
use super::*;

// tex/limits.go setLimits replaces a scripted owner's family, retaining
// children rather than the old wrapper's attributes and properties.
fn convert_limits_family(atom: &mut Atom, enabled: bool) {
    match atom {
        Atom::ScriptPolicy { body, .. } | Atom::NamedFunction(body) => {
            convert_limits_family(body, enabled);
        }
        Atom::Multiscripts {
            base,
            post,
            prescript: true,
            ..
        } if enabled => {
            let base = std::mem::replace(base, Box::new(Atom::Sequence(Vec::new())));
            let [sub, sup] = post.take().unwrap_or_else(multiscripts::empty_pair);
            let present = |child: Box<Atom>| {
                (!matches!(child.as_ref(), Atom::EmptyNode("none"))).then_some(child)
            };
            let (sub, sup) = (present(sub), present(sup));
            // cleanSubSup removes a parser-created generic owner with no
            // surviving right slots. Prescripts belong to the discarded family.
            *atom = if sub.is_none() && sup.is_none() {
                *base
            } else {
                Atom::Scripts {
                    base,
                    sub,
                    sup,
                    limits: true,
                    accent: None,
                    eager: true,
                    leading_space: 0.,
                }
            };
        }
        Atom::Scripts {
            limits,
            accent,
            eager,
            sub,
            sup,
            leading_space,
            ..
        } if *limits != enabled => {
            // Only parser-created compact scripts are expanded to full slots
            // before setLimits. An authored mover's child 1 stays in slot 1.
            if !*eager && *limits && sub.is_none() {
                *sub = sup.take();
            }
            *limits = enabled;
            *accent = None;
            *eager = true;
            *leading_space = 0.;
        }
        _ => {}
    }
}

// Pinned tex/limits.go always writes movablelimits=false on coreMO, even
// when the operator that owns the limits command is a transparent wrapper.
fn disable_core_movable_limits(atom: &mut Atom) {
    match atom {
        Atom::ScriptPolicy {
            movable: Some(value),
            ..
        } => *value = false,
        Atom::ScriptPolicy { body, .. }
        | Atom::Ord(body)
        | Atom::Operator(body)
        | Atom::AutoOperator(body)
        | Atom::MathMLSpace { body, .. }
        | Atom::NamedFunction(body)
        | Atom::Class { body, .. }
        | Atom::CrampedStyle(body, _)
        | Atom::DeclaredStyle { body, .. }
        | Atom::DeclaredSize { body, .. }
        | Atom::Color { body, .. }
        | Atom::Padded { body, .. }
        | Atom::Phantom(body)
        | Atom::Fraction(body, _)
        | Atom::GeneralFraction(body, _, _)
        | Atom::Scripts { base: body, .. }
        | Atom::Multiscripts { base: body, .. } => disable_core_movable_limits(body),
        Atom::Sequence(children) if row_core(children).is_some() => {
            let index = row_core(children).unwrap();
            disable_core_movable_limits(&mut children[index]);
        }
        _ => {
            let moves = script_moves(atom);
            let body = std::mem::replace(atom, Atom::Sequence(Vec::new()));
            *atom = Atom::ScriptPolicy {
                body: Box::new(body),
                moves,
                movable: Some(false),
                movable_property: false,
            };
        }
    }
}

fn row_core(children: &[Atom]) -> Option<usize> {
    let mut content = children
        .iter()
        .enumerate()
        .filter(|(_, child)| !operator_dictionary::flag_spacelike(child));
    let (index, child) = content.next()?;
    (content.next().is_none() && operator_dictionary::embellished(child)).then_some(index)
}

// A movesupsub property belongs to the actual owner, including the false
// value. It must not leak out through a TeXAtom or a non-embellished row.
fn own_moves(atom: &Atom) -> bool {
    match atom {
        Atom::ScriptPolicy { .. } | Atom::MovesScripts(_) | Atom::Class { class: 1, .. } => true,
        Atom::NamedFunction(body) => own_moves(body),
        Atom::Scripts {
            base, eager: true, ..
        } => own_moves(base),
        _ => false,
    }
}

// limitsCore delegates through layout nodes and through an embellished row's
// core index. A non-embellished row is the core itself, regardless of its first
// descendant's class or movesupsub property.
fn core_is_operator(atom: &Atom) -> bool {
    match atom {
        Atom::Ord(a)
        | Atom::MovesScripts(a)
        | Atom::ScriptPolicy { body: a, .. }
        | Atom::MathMLSpace { body: a, .. }
        | Atom::NamedFunction(a)
        | Atom::Class { body: a, .. }
        | Atom::CrampedStyle(a, _)
        | Atom::DeclaredStyle { body: a, .. }
        | Atom::DeclaredSize { body: a, .. }
        | Atom::Color { body: a, .. }
        | Atom::Padded { body: a, .. }
        | Atom::Phantom(a)
        | Atom::Fraction(a, _)
        | Atom::GeneralFraction(a, _, _)
        | Atom::Scripts { base: a, .. }
        | Atom::Multiscripts { base: a, .. } => core_is_operator(a),
        Atom::Sequence(v) | Atom::InferredRow(v) | Atom::ExplicitRow(v) => {
            row_core(v).is_some_and(|index| core_is_operator(&v[index]))
        }
        // Operator marks a token's OP class; a Class child instead owns a
        // real TeXAtom whose core delegates to its inferred row.
        Atom::Operator(a) if !matches!(a.as_ref(), Atom::Class { .. }) => true,
        Atom::Operator(a) => core_is_operator(a),
        // nodes.go:token initializes raw mo classes with operatorClass,
        // before applyOperatorInheritance consults the full dictionary.
        Atom::RawOperator { code, texclass, .. } => texclass.map_or_else(
            || {
                matches!(
                    *code,
                    0x2211 | 0x220F | 0x222B | 0x222C | 0x222D | 0x2210 | 0x22C3 | 0x22C2
                )
            },
            |class| class == 1,
        ),
        _ => atom_tex_class(atom) == Some(1),
    }
}

fn limits_allowed(atom: &Atom) -> bool {
    own_moves(atom) || core_is_operator(atom)
}

// setLimits validates before replacing the owner, then writes its own policy
// and the delegated core's attribute. Repeated commands replace the policy
// rather than nesting another owner around it.
pub(super) fn set_limits(base: Option<Atom>, name: &str) -> Result<Atom, String> {
    let Some(mut base) = base.filter(limits_allowed) else {
        return Err(format!("\\{name} is allowed only on operators"));
    };
    let enabled = name == "limits";
    convert_limits_family(&mut base, enabled);
    let core_here = script_core_is_root(&base);
    disable_core_movable_limits(&mut base);
    let body = match base {
        Atom::ScriptPolicy { body, .. } => body,
        body => Box::new(body),
    };
    Ok(Atom::ScriptPolicy {
        body,
        moves: enabled,
        movable: core_here.then_some(false),
        movable_property: false,
    })
}

// Mirrors which original node remains scriptBaseCore after transparent
// single-child rows/styles/TeXAtoms are unwrapped. True preserves attributes
// belonging to that core; false means the modified owner is outside the core.
fn script_core_is_root(atom: &Atom) -> bool {
    match atom {
        Atom::ScriptPolicy { movable, .. } => movable.is_some(),
        Atom::AutoOperator(a)
        | Atom::Operator(a)
        | Atom::MovesScripts(a)
        | Atom::NamedFunction(a) => script_core_is_root(a),
        Atom::Ord(_)
        | Atom::Class { .. }
        | Atom::CrampedStyle(_, _)
        | Atom::DeclaredStyle { .. }
        | Atom::DeclaredSize { .. }
        | Atom::Color { .. }
        | Atom::Padded { .. }
        | Atom::Phantom(_) => false,
        Atom::Sequence(a) => a.len() != 1,
        _ => true,
    }
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn limits_convert_existing_script_families_without_reassigning_generated_slots() {
        // Pinned limits_test.go TestLimitsEagerScriptOriginAndCleanup:
        // a generated upper slot survives a round trip between families.
        for (changed, direct) in [
            (r"\sum_i\nolimits^n", r"\sum\nolimits_i^n"),
            (r"\sum^i\nolimits\limits_n", r"\sum\limits_n^i"),
            (r"\prescript{a}{b}{\sum}\limits^n", r"\sum\limits^n"),
            (
                r"\prescript{a}{b}{\sum}\nolimits^n",
                r"\prescript{a}{b}{\sum\nolimits}^n",
            ),
        ] {
            assert_eq!(
                render(changed).unwrap(),
                render(direct).unwrap(),
                "{changed}"
            );
        }
        // Authored mover child 1 becomes the lower slot after replacement.
        let converted = render(r"\overline{\sum}\nolimits^n").unwrap();
        assert!(converted.contains("data-mml-node=\"msubsup\""));
        assert!(!converted.contains("data-mml-node=\"mover\""));
        // Without replacement an authored mover remains the base of a new
        // mover, even when its explicit movesupsub property is true.
        let nested = render(r"\overline{\sum}\limits^n").unwrap();
        assert_eq!(nested.matches("data-mml-node=\"mover\"").count(), 2);
    }

    #[test]
    fn limits_use_the_owners_property_and_the_embellished_rows_actual_core() {
        for command in ["limits", "nolimits"] {
            for base in [r"{\mathop{x}}", r"{\mathop{x+y}}", r"{x\sum}"] {
                let expression = format!("{base}\\{command}^n");
                assert_eq!(
                    render(&expression).unwrap(),
                    cd::render_error(&format!("\\{command} is allowed only on operators")),
                    "{expression}"
                );
            }
        }
        // Spacelike siblings do not make an operator row non-embellished.
        // A false, but present, own movesupsub property remains eligible.
        for expression in [r"{\,\sum}\limits^n", r"\mathop{x}\nolimits\limits^n"] {
            let svg = render(expression).unwrap();
            assert!(!svg.contains("data-mml-node=\"merror\""), "{expression}");
            assert!(svg.contains("data-mml-node=\"mover\""), "{expression}");
        }
    }
}
