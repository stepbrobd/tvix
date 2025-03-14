use bstr::B;

use super::*;

#[test]
fn test_empty_attrs() {
    let attrs = NixAttrs::construct(0, vec![])
        .expect("empty attr construction should succeed")
        .unwrap();

    assert!(
        matches!(attrs.0.as_ref(), AttrsRep::Empty),
        "empty attribute set should use optimised representation"
    );
}

#[test]
fn test_simple_attrs() {
    let attrs = NixAttrs::construct(1, vec![Value::from("key"), Value::from("value")])
        .expect("simple attr construction should succeed")
        .unwrap();

    assert!(
        matches!(attrs.0.as_ref(), AttrsRep::Map(_)),
        "simple attribute set should use map representation",
    )
}

#[test]
fn test_kv_attrs() {
    let name_val = Value::from("name");
    let value_val = Value::from("value");
    let meaning_val = Value::from("meaning");
    let forty_two_val = Value::Integer(42);

    let kv_attrs = NixAttrs::construct(
        2,
        vec![
            value_val,
            forty_two_val.clone(),
            name_val,
            meaning_val.clone(),
        ],
    )
    .expect("constructing K/V pair attrs should succeed")
    .unwrap();

    match kv_attrs.0.as_ref() {
        AttrsRep::KV { name, value }
            if name.to_str().unwrap() == meaning_val.to_str().unwrap()
                || value.to_str().unwrap() == forty_two_val.to_str().unwrap() => {}

        _ => panic!(
            "K/V attribute set should use optimised representation, but got {:?}",
            kv_attrs
        ),
    }
}

#[test]
fn test_empty_attrs_iter() {
    let attrs = NixAttrs::construct(0, vec![]).unwrap().unwrap();
    assert!(attrs.iter().next().is_none());
}

#[test]
fn test_kv_attrs_iter() {
    let name_val = Value::from("name");
    let value_val = Value::from("value");
    let meaning_val = Value::from("meaning");
    let forty_two_val = Value::Integer(42);

    let kv_attrs = NixAttrs::construct(
        2,
        vec![
            value_val,
            forty_two_val.clone(),
            name_val,
            meaning_val.clone(),
        ],
    )
    .expect("constructing K/V pair attrs should succeed")
    .unwrap();

    let mut iter = kv_attrs.iter().collect::<Vec<_>>().into_iter();
    let (k, v) = iter.next().unwrap();
    assert!(*k == *NAME);
    assert!(v.to_str().unwrap() == meaning_val.to_str().unwrap());
    let (k, v) = iter.next().unwrap();
    assert!(*k == *VALUE);
    assert!(v.as_int().unwrap() == forty_two_val.as_int().unwrap());
    assert!(iter.next().is_none());
}

#[test]
fn test_map_attrs_iter() {
    let attrs = NixAttrs::construct(1, vec![Value::from("key"), Value::from("value")])
        .expect("simple attr construction should succeed")
        .unwrap();

    let mut iter = attrs.iter().collect::<Vec<_>>().into_iter();
    let (k, v) = iter.next().unwrap();
    assert!(k == &NixString::from("key"));
    assert_eq!(v.to_str().unwrap(), B("value"));
    assert!(iter.next().is_none());
}
