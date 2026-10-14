use super::*;

fn new(kind: SecretType) -> NewSecret {
    NewSecret::new(ClusterId::new("ams"), "shop", "db", kind)
}

fn with(mut secret: NewSecret, key: &str, bytes: &[u8]) -> NewSecret {
    secret.entries.insert(key.to_owned(), Value::new(bytes.to_vec()));
    secret
}

#[test]
fn each_known_type_reads_back_from_its_name_and_an_other_name_stays() {
    for kind in SecretType::KNOWN {
        assert_eq!(SecretType::named(kind.name()), kind);
    }
    assert_eq!(SecretType::named(""), SecretType::Opaque, "the server gives a Secret with no type this one");
    assert_eq!(SecretType::named("helm.sh/release.v1").name(), "helm.sh/release.v1");
}

#[test]
fn a_new_secret_starts_with_the_keys_of_its_type_and_each_hint_tells_what_the_server_refuses() {
    let tls = new(SecretType::Tls);
    assert_eq!(tls.entries.keys().collect::<Vec<_>>(), ["tls.crt", "tls.key"]);
    assert_eq!(
        tls.hints(),
        [
            "the type kubernetes.io/tls needs a value of `tls.crt`",
            "the type kubernetes.io/tls needs a value of `tls.key`"
        ]
    );
    let filled = with(with(tls, "tls.crt", b"no certificate"), "tls.key", b"key");
    assert_eq!(filled.hints(), ["`tls.crt`: the value has no PEM certificate"]);

    let pull = with(new(SecretType::DockerConfigJson), ".dockerconfigjson", br#"{"auths":{"#);
    assert_eq!(pull.hints(), ["`.dockerconfigjson`: the JSON does not read: an error at line 1, column 10"]);
    let good = with(new(SecretType::DockerConfigJson), ".dockerconfigjson", br#"{"auths":{}}"#);
    assert_eq!(good.hints(), Vec::<String>::new());

    let basic = new(SecretType::BasicAuth);
    assert_eq!(basic.hints(), ["a basic-auth Secret needs a username or a password"]);
    assert_eq!(with(basic, "username", b"admin").hints(), Vec::<String>::new(), "one of the two is enough");

    let token = new(SecretType::ServiceAccountToken);
    assert_eq!(token.annotations.keys().collect::<Vec<_>>(), [SERVICE_ACCOUNT_ANNOTATION]);
    assert_eq!(token.hints(), ["a token Secret needs the annotation `kubernetes.io/service-account.name`"]);

    let opaque = with(NewSecret { name: String::new(), ..new(SecretType::Opaque) }, "a b", b"1");
    assert_eq!(
        opaque.hints(),
        ["`a b`: a key has only letters, digits, `-`, `_` and `.`", "a Secret needs a name and a namespace"]
    );
    let ssh = new(SecretType::SshAuth);
    assert_eq!(ssh.hints(), ["the type kubernetes.io/ssh-auth needs a value of `ssh-privatekey`"]);
}
