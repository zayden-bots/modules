use ai::persona::Persona;

const PERSONA: Persona = Persona {
    name: "Tester",
    character: "CHARACTER-MARKER",
    siblings: "SIBLINGS-MARKER",
};

#[test]
fn the_prompt_is_assembled_in_order() {
    let prompt = PERSONA.system_prompt("WORLD-MARKER", 250);

    assert!(
        prompt
            .starts_with("[Word Limit: 250 words]\nCHARACTER-MARKER\n\nYOUR FAMILY")
    );

    let positions: Vec<usize> = [
        "CHARACTER-MARKER",
        "YOUR FAMILY",
        "SIBLINGS-MARKER",
        "WORLD-MARKER",
        "HOW YOU TALK",
    ]
    .into_iter()
    .map(|marker| prompt.find(marker).unwrap())
    .collect();

    assert!(positions.is_sorted());
    assert!(prompt.contains("\n\nSIBLINGS-MARKER\n\nWORLD-MARKER\n\nHOW YOU TALK"));
}

#[test]
fn the_prompt_carries_the_shared_blocks() {
    let prompt = PERSONA.system_prompt("WORLD-MARKER", 100);

    assert!(prompt.contains("YOUR FAMILY"));
    assert!(prompt.contains("STAY IN CHARACTER"));
}
