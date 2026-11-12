//! The add-a-source panel's own rules, tested without a network: what it asks
//! the worker for, which answer belongs to the step it is on, and where a
//! failure leaves it.

use super::{AddFailure, AddFlow, DiscoveredServer, FlowInput, Stage, ValidatedServer};
use crate::sources::JellyfinServer;
use crate::watch::JfCmd;

fn server(id: &str) -> ValidatedServer {
    ValidatedServer {
        server_id: id.to_owned(),
        name: format!("{id} box"),
        base_url: format!("http://192.168.1.5:8096/{id}"),
        version: "10.10.4".to_owned(),
    }
}

fn source(id: &str) -> JellyfinServer {
    JellyfinServer {
        server_id: id.to_owned(),
        name: format!("{id} box"),
        base_url: format!("http://192.168.1.5:8096/{id}"),
        user_id: Some("someone".to_owned()),
        user_name: Some("someone".to_owned()),
        added_at: 1_700_000_000,
    }
}

/// Drive the panel to the step where a known server can be signed in to.
fn ready(mut flow: AddFlow) -> AddFlow {
    flow.apply(FlowInput::Address("192.168.1.5:8096".to_owned()));
    flow.apply(FlowInput::SubmitAddress);
    flow.apply(FlowInput::Validated(server("srv-1")));
    flow
}

#[test]
fn a_fresh_panel_is_already_asking_the_network() {
    let flow = AddFlow::open();
    assert_eq!(flow.stage, Stage::Address);
    assert!(flow.discovering, "the panel opens by asking");
    assert!(flow.error.is_none(), "asking is not a failure");
}

#[test]
fn a_blank_address_is_refused_before_anything_is_asked() {
    let mut flow = AddFlow::open();
    assert_eq!(flow.apply(FlowInput::SubmitAddress), None);
    assert_eq!(flow.stage, Stage::Address);
    assert_eq!(flow.error, Some(AddFailure::MissingAddress));
}

#[test]
fn submitting_an_address_asks_the_worker_and_waits() {
    let mut flow = AddFlow::open();
    flow.apply(FlowInput::Address("192.168.1.5:8096".to_owned()));
    let asked = flow.apply(FlowInput::SubmitAddress);
    assert!(
        matches!(&asked, Some(JfCmd::Validate { address }) if address == "192.168.1.5:8096"),
        "the panel asks about what its field holds, not what it guesses: {asked:?}"
    );
    assert_eq!(flow.stage, Stage::Checking);
}

#[test]
fn retyping_while_an_answer_is_coming_leaves_that_step() {
    let mut flow = AddFlow::open();
    flow.apply(FlowInput::Address("192.168.1.5:8096".to_owned()));
    flow.apply(FlowInput::SubmitAddress);
    assert_eq!(flow.stage, Stage::Checking);
    flow.apply(FlowInput::Address("jellyfin.home:8096".to_owned()));
    assert_eq!(
        flow.stage,
        Stage::Address,
        "an address being typed is a new attempt"
    );
    assert!(flow.error.is_none());
}

#[test]
fn an_answer_to_a_step_already_left_is_dropped() {
    let mut flow = ready(AddFlow::open());
    // The panel left Checking when the first answer arrived; a second answer to
    // an earlier attempt cannot move it again.
    assert_eq!(flow.apply(FlowInput::Validated(server("srv-2"))), None);
    assert_eq!(flow.stage, Stage::Ready);
    assert_eq!(
        flow.server.as_ref().map(|held| held.server_id.as_str()),
        Some("srv-1"),
        "a late answer does not rewrite the server the panel is showing"
    );
}

#[test]
fn an_empty_broadcast_is_a_choice_list_of_none() {
    let mut flow = AddFlow::open();
    assert!(matches!(
        flow.apply(FlowInput::Discover),
        Some(JfCmd::Discover)
    ));
    assert!(flow.discovering);
    assert_eq!(flow.apply(FlowInput::Discovered(Vec::new())), None);
    assert!(!flow.discovering);
    assert!(flow.error.is_none(), "nothing answering is not a failure");
    assert_eq!(flow.stage, Stage::Address);
}

#[test]
fn a_chosen_server_becomes_the_address_without_another_ask() {
    let mut flow = AddFlow::open();
    flow.apply(FlowInput::Discovered(vec![DiscoveredServer {
        name: "Living room".to_owned(),
        address: "http://192.168.1.9:8096".to_owned(),
    }]));
    assert_eq!(
        flow.discovered.first().map(|found| found.address.as_str()),
        Some("http://192.168.1.9:8096")
    );
}

#[test]
fn choosing_a_code_shows_a_wait_before_the_code_arrives() {
    let mut flow = ready(AddFlow::open());
    assert!(matches!(
        flow.apply(FlowInput::ChooseQuickConnect),
        Some(JfCmd::QuickConnect)
    ));
    assert_eq!(flow.stage, Stage::Pairing);
    assert!(flow.code.is_empty(), "the server has not named one yet");
    assert_eq!(flow.apply(FlowInput::Code("123456".to_owned())), None);
    assert_eq!(flow.code, "123456");
}

#[test]
fn switching_method_stops_a_code_still_waiting_but_not_a_idle_panel() {
    let mut pairing = ready(AddFlow::open());
    pairing.apply(FlowInput::ChooseQuickConnect);
    pairing.apply(FlowInput::Code("123456".to_owned()));
    let abandoned = pairing.apply(FlowInput::ChoosePassword);
    assert!(
        matches!(abandoned, Some(JfCmd::Stop)),
        "the code it abandoned would still have been approved otherwise: {abandoned:?}"
    );
    assert_eq!(pairing.stage, Stage::Password);

    let mut idle = ready(AddFlow::open());
    assert_eq!(idle.apply(FlowInput::ChoosePassword), None);
    assert_eq!(idle.stage, Stage::Password);
}

#[test]
fn the_password_goes_once_and_then_leaves_the_flow() {
    let mut flow = ready(AddFlow::open());
    flow.apply(FlowInput::ChoosePassword);
    flow.apply(FlowInput::Username("someone".to_owned()));
    flow.apply(FlowInput::Password("hunter2".to_owned()));
    let asked = flow.apply(FlowInput::SubmitPassword);
    assert!(
        matches!(
            &asked,
            Some(JfCmd::SignIn { username, password })
                if username == "someone" && password == "hunter2"
        ),
        "the step is sent what was typed: {asked:?}"
    );
    assert!(flow.password.is_empty(), "only the request holds it");
    assert_eq!(flow.stage, Stage::SigningIn);
    // Pressing again while the first answer is coming is a second attempt, not a
    // queue: what the field holds now is what goes, and only the last answer to
    // arrive is kept.
    assert_eq!(
        flow.apply(FlowInput::SubmitPassword),
        Some(JfCmd::SignIn {
            username: "someone".to_owned(),
            password: String::new(),
        })
    );
}

#[test]
fn a_blank_username_is_refused_without_asking() {
    let mut flow = ready(AddFlow::open());
    flow.apply(FlowInput::ChoosePassword);
    flow.apply(FlowInput::Password("hunter2".to_owned()));
    assert_eq!(flow.apply(FlowInput::SubmitPassword), None);
    assert_eq!(flow.error, Some(AddFailure::MissingUsername));
    assert_eq!(flow.stage, Stage::Password);
}

#[test]
fn every_failure_lands_where_the_user_can_act_again() {
    let cases = [
        (Stage::Address, Stage::Address),
        (Stage::Checking, Stage::Address),
        (Stage::Ready, Stage::Ready),
        (Stage::Pairing, Stage::Ready),
        (Stage::Password, Stage::Password),
        (Stage::SigningIn, Stage::Password),
    ];
    for (stage, lands) in cases {
        let mut flow = AddFlow::open();
        flow.stage = stage;
        assert_eq!(
            flow.apply(FlowInput::Failed(AddFailure::Unreachable)),
            None,
            "a failure asks for nothing"
        );
        assert_eq!(flow.stage, lands, "failing from {stage:?}");
        assert!(!flow.stage.is_busy(), "no step may trap the panel");
        let message = flow
            .error
            .as_ref()
            .map_or_else(String::new, AddFailure::message);
        assert!(
            message.chars().count() > 20,
            "failing from {stage:?} says too little: {message}"
        );
    }
}

#[test]
fn a_code_that_arrives_after_its_step_is_gone_stays_gone() {
    let mut flow = ready(AddFlow::open());
    flow.apply(FlowInput::ChooseQuickConnect);
    flow.apply(FlowInput::Code("123456".to_owned()));
    flow.apply(FlowInput::Failed(AddFailure::Expired));
    assert_eq!(flow.stage, Stage::Ready);
    flow.apply(FlowInput::Code("999999".to_owned()));
    assert_eq!(flow.stage, Stage::Ready);
    assert_ne!(
        flow.code, "999999",
        "a code for a step the panel already left fills nothing in"
    );
}

#[test]
fn an_added_source_ends_the_panel_and_names_its_row() {
    let mut flow = ready(AddFlow::open());
    flow.apply(FlowInput::ChooseQuickConnect);
    flow.apply(FlowInput::Code("123456".to_owned()));
    assert_eq!(flow.apply(FlowInput::Added(source("srv-9"))), None);
    assert_eq!(flow.stage, Stage::Added);
    assert_eq!(
        flow.server.as_ref().map(|held| held.server_id.as_str()),
        Some("srv-9")
    );
}

#[test]
fn every_failure_tells_the_user_what_to_do_next() {
    let failures = [
        AddFailure::MissingAddress,
        AddFailure::MissingUsername,
        AddFailure::BadAddress,
        AddFailure::Unreachable,
        AddFailure::HttpStatus(404),
        AddFailure::WizardIncomplete,
        AddFailure::QuickConnectDisabled,
        AddFailure::Expired,
        AddFailure::Rejected,
        AddFailure::ServerShape,
        AddFailure::Identity,
        AddFailure::Storage,
    ];
    for failure in failures {
        let message = failure.message();
        assert!(message.len() > 20, "{failure:?} says too little: {message}");
        assert!(
            message.ends_with('.') || message.ends_with('?'),
            "{failure:?} is not a sentence: {message}"
        );
    }
    assert!(
        AddFailure::HttpStatus(404).message().contains("HTTP 404"),
        "the status the server gave belongs in the message"
    );
}
