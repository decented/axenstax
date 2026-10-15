use super::*;

const SPECIES: [MobType; 7] = [
    MobType::Cat,
    MobType::Fox,
    MobType::Crab,
    MobType::Reindeer,
    MobType::Parrot,
    MobType::Donkey,
    MobType::Mule,
];

#[test]
fn animal_polish_silhouettes_match_their_species() {
    for kind in [MobType::Donkey, MobType::Mule] {
        let parts = mob_model(kind);
        let head_top = parts[2].origin.y + parts[2].size.y * 0.5;
        assert!(parts.len() >= 9, "{kind:?} needs two long ears");
        for ear in &parts[7..9] {
            assert!(
                ear.origin.y - ear.size.y * 0.5 <= head_top,
                "{kind:?} ears must join the head without a sky gap"
            );
        }
    }
    let crab = mob_model(MobType::Crab);
    assert!(
        crab[0].size.x > crab[0].size.y * 1.5,
        "crab needs a low, wide shell"
    );
    assert_eq!(
        crab.iter().filter(|p| p.animated).count(),
        8,
        "eight walking legs"
    );
    let cat = mob_model(MobType::Cat);
    assert!(
        cat[0].origin.y + cat[0].size.y * 0.5 <= crate::mob::mob_def(MobType::Cat).height,
        "cat trunk fits the cat's existing interaction height"
    );
    assert!(cat[2].size.z < cat[1].size.z * 0.5, "short feline muzzle");
    let deer = mob_model(MobType::Reindeer);
    let head_top = deer[2].origin.y + deer[2].size.y * 0.5;
    assert!(
        deer.iter()
            .skip(7)
            .filter(|p| p.origin.y > head_top)
            .count()
            >= 6,
        "branched antlers distinguish reindeer from horses"
    );
}

#[test]
fn animal_polish_faces_are_on_the_front_of_the_head_only() {
    for (kind, head_idx, face) in [
        (MobType::Cat, 1, TEX_SHEEP_HEAD_FRONT),
        (MobType::Fox, 1, TEX_SHEEP_HEAD_FRONT),
        (MobType::Reindeer, 2, TEX_COW_HEAD_FRONT),
        (MobType::Donkey, 2, TEX_COW_HEAD_FRONT),
        (MobType::Mule, 2, TEX_COW_HEAD_FRONT),
    ] {
        let expected = crate::texture_gen::tinted_layer(kind, face).unwrap();
        let faces = mob_model(kind)[head_idx].tex_faces;
        assert_eq!(faces[5], expected, "{kind:?} face points forward (-Z)");
        assert!(
            !faces[..5].contains(&expected),
            "{kind:?} has no face on its crown or rear"
        );
    }
    let bird = &mob_model(MobType::Parrot)[1];
    let cheek = crate::texture_gen::tinted_layer(MobType::Parrot, TEX_CHICKEN_HEAD).unwrap();
    assert_eq!(
        bird.tex_faces[..2],
        [cheek; 2],
        "parrot has an eye on each cheek"
    );
    assert!(
        !bird.tex_faces[2..].contains(&cheek),
        "no repeated cheek mask around the beak or rear"
    );
}

#[test]
fn animal_polish_live_mesh_preserves_baby_scale_and_finite_walk_poses() {
    let overrides = crate::override_registry::OverrideRegistry::new();
    for kind in SPECIES {
        let mut ecs = hecs::World::new();
        let id = crate::entity::spawn_mob(&mut ecs, kind, Vec3::ZERO);
        let camera = Vec3::new(3.0, 2.0, -4.0);
        let adult = build_entity_model_vertices(&ecs, 0, camera, &overrides, |_| (0.3, 0.8));
        assert_eq!(adult.len(), mob_model(kind).len() * 36);
        assert!(adult
            .iter()
            .all(|v| v.tex_layer < crate::texture_gen::texture_count()));
        ecs.insert_one(id, crate::breeding::Baby { adult_at_tick: 100 })
            .unwrap();
        let baby = build_entity_model_vertices(&ecs, 0, camera, &overrides, |_| (0.3, 0.8));
        assert_eq!(baby.len(), adult.len());
        for (a, b) in adult.iter().zip(&baby) {
            for axis in 0..3 {
                assert!(
                    (b.position[axis] - a.position[axis] * crate::breeding::BABY_RENDER_SCALE)
                        .abs()
                        < 1e-5,
                    "{kind:?} baby scales every appended part"
                );
            }
        }
        ecs.get::<&mut Velocity>(id).unwrap().0.x = 1.0;
        for tick in [0, 4, 9, 17] {
            let walk = build_entity_model_vertices(&ecs, tick, camera, &overrides, |_| (0.3, 0.8));
            assert!(walk
                .iter()
                .all(|v| v.position.iter().chain(&v.normal).all(|x| x.is_finite())));
        }
    }
}

/// Uses the existing screenshot path and actual live entity mesh. Set the
/// output directory explicitly so a test never writes to a player's saves.
#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "headless GPU animal render; set AXENSTAX_ANIMAL_SHOTS"]
fn animal_polish_render_species() {
    let output =
        std::env::var("AXENSTAX_ANIMAL_SHOTS").expect("explicit isolated output directory");
    std::fs::create_dir_all(&output).unwrap();
    for name in ["cat", "fox", "crab", "reindeer", "parrot", "donkey", "mule"] {
        crate::run_screenshot(&[
            "--screenshot".into(),
            format!("{output}/{name}.png"),
            "--mob".into(),
            name.into(),
            "--width".into(),
            "800".into(),
            "--height".into(),
            "700".into(),
            "--time".into(),
            "12000".into(),
        ]);
        let zoom = if matches!(name, "reindeer" | "donkey" | "mule") {
            "150"
        } else {
            "280"
        };
        crate::run_screenshot(&[
            "--screenshot".into(),
            format!("{output}/{name}-detail.png"),
            "--mob".into(),
            name.into(),
            "--width".into(),
            "800".into(),
            "--height".into(),
            "700".into(),
            "--time".into(),
            "12000".into(),
            "--zoom".into(),
            zoom.into(),
        ]);
    }
    for (label, name, extra) in [
        (
            "crab-walking",
            "crab",
            vec!["--walk-tick", "9", "--zoom", "280"],
        ),
        ("cat-baby", "cat", vec!["--baby", "--zoom", "280"]),
        (
            "reindeer-night",
            "reindeer",
            vec!["--time", "0", "--zoom", "150"],
        ),
        (
            "parrot-back",
            "parrot",
            vec!["--angle", "180", "--zoom", "280"],
        ),
    ] {
        let mut args = vec![
            "--screenshot".into(),
            format!("{output}/{label}.png"),
            "--mob".into(),
            name.into(),
            "--width".into(),
            "800".into(),
            "--height".into(),
            "700".into(),
        ];
        args.extend(extra.into_iter().map(String::from));
        crate::run_screenshot(&args);
    }
}
