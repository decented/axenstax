use super::*;

#[test]
fn roster_polish_human_tiers_have_distinct_silhouettes_and_materials() {
    let kinds = [
        MobType::Villager,
        MobType::Peddler,
        MobType::Brigand,
        MobType::Marauder,
        MobType::Berserker,
        MobType::Knight,
    ];
    for (i, kind) in kinds.iter().enumerate() {
        let a = mob_model(*kind);
        for other in &kinds[i + 1..] {
            let b = mob_model(*other);
            assert!(
                a.len() != b.len()
                    || a.iter().zip(b).any(|(a, b)| a.size != b.size
                        || a.origin != b.origin
                        || a.tex_faces != b.tex_faces),
                "{kind:?} and {other:?} must read differently"
            );
        }
    }
}

#[test]
fn roster_polish_npcs_face_the_direction_they_walk() {
    for (kind, face) in [
        (MobType::Villager, TEX_VILLAGER_HEAD_FRONT),
        (MobType::Peddler, TEX_WANDERING_HEAD_FRONT),
    ] {
        let head = &mob_model(kind)[1];
        assert_eq!(head.tex_faces[5], face, "{kind:?} looks forward (-Z)");
        assert!(!head.tex_faces[..5].contains(&face));
    }
    assert_eq!(SATOSHI_MODEL[1].tex_faces[5], TEX_SATOSHI_HEAD_FRONT);
    assert!(
        SATOSHI_MODEL[SATOSHI_GLOW_PART].origin.z < 0.0,
        "amulet is on the chest"
    );
    assert!(SATOSHI_MODEL[6].origin.z > 0.0, "cloak is behind the back");
}

#[test]
fn roster_polish_aquatic_anatomy_and_forward_direction() {
    for kind in [MobType::Squid, MobType::GlowSquid] {
        let m = mob_model(kind);
        assert_eq!(
            m.iter().filter(|p| p.animated).count(),
            8,
            "eight moving arms for {kind:?}"
        );
        assert!(m.iter().all(|p| p.origin.y - p.size.y * 0.5 >= -0.001));
    }
    for kind in [MobType::Fish, MobType::Shark] {
        let m = mob_model(kind);
        let tail = if kind == MobType::Fish { &m[1] } else { &m[2] };
        assert!(tail.origin.z > m[0].origin.z, "{kind:?} tail trails +Z");
        assert!(
            m.len() >= 8,
            "{kind:?} needs eyes, fins and a connected tail"
        );
    }
}

#[test]
fn roster_polish_attached_detail_shares_the_limb_transform() {
    for (kind, part_count) in [
        (MobType::Cow, 6),
        (MobType::Pig, 6),
        (MobType::Chicken, 6),
        (MobType::Horse, 7),
        (MobType::Knight, 6),
    ] {
        let m = mob_model(kind);
        assert!(m.len() > part_count, "{kind:?} needs silhouette detail");
        for detail in m.iter().skip(part_count).filter(|p| p.animated) {
            assert!(
                m[..part_count].iter().any(|limb| limb.animated
                    && limb.pivot == detail.pivot
                    && limb.phase == detail.phase),
                "{kind:?} detail must swing with its parent limb"
            );
        }
    }
}

#[test]
fn roster_polish_every_live_mesh_walks_scales_and_accepts_workshop_reskins() {
    use crate::override_registry::{AuthoredFaces, MobPartKey, NamedDesign, OverrideRegistry};
    let empty = OverrideRegistry::new();
    let camera = Vec3::new(3.0, 2.0, -4.0);
    for def in crate::mob::MOBS.iter() {
        let kind = def.mob_type;
        let model = mob_model(kind);
        assert!(!model.is_empty() && model.len() <= u8::MAX as usize);
        assert!(model
            .iter()
            .all(|p| p.size.is_finite() && p.size.min_element() > 0.0));
        let mut ecs = hecs::World::new();
        let id = crate::entity::spawn_mob(&mut ecs, kind, Vec3::ZERO);
        let adult = build_entity_model_vertices(&ecs, 0, camera, &empty, |_| (0.3, 0.8));
        assert_eq!(adult.len(), model.len() * 36);
        assert!(adult
            .iter()
            .all(|v| v.tex_layer < crate::texture_gen::texture_count()));
        ecs.insert_one(id, crate::breeding::Baby { adult_at_tick: 100 })
            .unwrap();
        let baby = build_entity_model_vertices(&ecs, 0, camera, &empty, |_| (0.3, 0.8));
        for (a, c) in adult.iter().zip(&baby) {
            for axis in 0..3 {
                assert!(
                    (c.position[axis] - a.position[axis] * crate::breeding::BABY_RENDER_SCALE)
                        .abs()
                        < 1e-5,
                    "{kind:?}: every accessory scales with its juvenile"
                );
            }
        }
        ecs.remove_one::<crate::breeding::Baby>(id).unwrap();
        ecs.get::<&mut Velocity>(id).unwrap().0.x = 1.0;
        for tick in [0, 4, 9, 17] {
            let mesh = build_entity_model_vertices(&ecs, tick, camera, &empty, |_| (0.3, 0.8));
            assert!(mesh
                .iter()
                .all(|v| v.position.iter().chain(&v.normal).all(|x| x.is_finite())));
        }
        ecs.get::<&mut Velocity>(id).unwrap().0 = Vec3::ZERO;
        // Exercise an original slot and the final accessory through the real
        // override registry, both on a live mob and its Workshop mannequin.
        for part in [0, model.len() - 1] {
            let mut painted = OverrideRegistry::new();
            let base = crate::texture_gen::texture_count();
            painted.add_mob_design(
                MobPartKey {
                    mob: kind,
                    part: part as u8,
                },
                NamedDesign {
                    id: 0,
                    name: "Coat check".into(),
                    faces: Some(AuthoredFaces::solid([17, 94, 133, 255])),
                    micro_model: None,
                    author_npub: String::new(),
                    derivation_chain: Vec::new(),
                },
                base,
            );
            let mesh = build_entity_model_vertices(&ecs, 0, camera, &painted, |_| (0.3, 0.8));
            let mannequin =
                build_mob_mannequin_vertices(kind, Vec3::ZERO, 0.0, 1.0, false, 0.0, &painted);
            for mesh in [&mesh, &mannequin] {
                assert!(mesh[part * 36..(part + 1) * 36]
                    .iter()
                    .all(|v| v.tex_layer == base));
                assert_eq!(mesh.iter().filter(|v| v.tex_layer == base).count(), 36);
            }
        }
    }
}

#[test]
fn roster_polish_rear_heads_and_neck_junction_have_clean_surfaces() {
    let face_art = [
        TEX_VILLAGER_HEAD_FRONT,
        TEX_VILLAGER_HEAD_SIDE,
        TEX_WANDERING_HEAD_FRONT,
        TEX_WANDERING_HEAD_SIDE,
        TEX_SATOSHI_HEAD_FRONT,
        TEX_SATOSHI_HEAD_SIDE,
    ];
    for kind in [
        MobType::Villager,
        MobType::Peddler,
        MobType::Brigand,
        MobType::Marauder,
        MobType::Berserker,
        MobType::Knight,
    ] {
        let head = &mob_model(kind)[1];
        assert!(
            !face_art.contains(&head.tex_faces[2]),
            "{kind:?}: crown is hair/hood/helmet"
        );
        assert!(
            !face_art.contains(&head.tex_faces[4]),
            "{kind:?}: rear cannot reuse cheek/face artwork"
        );
    }
    assert!(!face_art.contains(&SATOSHI_MODEL[1].tex_faces[4]));
    let n = mob_model(MobType::Nostrich);
    let body_front = n[0].origin.z - n[0].size.z * 0.5;
    let neck_front = n[1].origin.z - n[1].size.z * 0.5;
    assert!(
        neck_front > body_front + 0.001,
        "neck is inset, not coplanar with the chest"
    );
    assert!(n[1].origin.y - n[1].size.y * 0.5 < n[0].origin.y + n[0].size.y * 0.5);
}
