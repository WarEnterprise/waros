use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::manifest::{
    Manifest, ManifestFile, SignatureEnvelope, SignedManifest, WarPackBundle, WarPackPayload,
};
use super::signature::{payload_digests, WARPKG_BOOTSTRAP_KEY_ID, WARPKG_SIGNATURE_SCHEME};

const QUANTUM_EXAMPLES_SIGNATURE: &str = include_str!("signatures/quantum-examples.sig.hex");
const CRYPTO_TOOLS_SIGNATURE: &str = include_str!("signatures/crypto-tools.sig.hex");
const NETWORK_UTILS_SIGNATURE: &str = include_str!("signatures/network-utils.sig.hex");
const SYSTEM_MONITOR_SIGNATURE: &str = include_str!("signatures/system-monitor.sig.hex");
const WAROS_DOCS_SIGNATURE: &str = include_str!("signatures/waros-docs.sig.hex");
const QUANTUM_BENCHMARKS_SIGNATURE: &str = include_str!("signatures/quantum-benchmarks.sig.hex");
const HELLO_WORLD_SIGNATURE: &str = include_str!("signatures/hello-world.sig.hex");
const WAR_SHELL_PLUGINS_SIGNATURE: &str = include_str!("signatures/war-shell-plugins.sig.hex");

pub fn built_in_packages() -> Vec<WarPackBundle> {
    vec![
        signed_package(
            QUANTUM_EXAMPLES_SIGNATURE,
            "quantum-examples",
            "Quantum example circuits",
            "quantum",
            vec![
                file(
                    "/usr/share/quantum/bell.qasm",
                    "bell.qasm",
                    false,
                    include_str!("../../../examples/qasm/bell.qasm"),
                ),
                file(
                    "/usr/share/quantum/ghz5.qasm",
                    "ghz5.qasm",
                    false,
                    include_str!("../../../examples/qasm/ghz5.qasm"),
                ),
                file(
                    "/usr/share/quantum/grover2.qasm",
                    "grover2.qasm",
                    false,
                    include_str!("../../../examples/qasm/grover2.qasm"),
                ),
                file(
                    "/usr/share/quantum/qft4.qasm",
                    "qft4.qasm",
                    false,
                    include_str!("../../../examples/qasm/qft4.qasm"),
                ),
            ],
            Vec::new(),
        ),
        signed_package(
            CRYPTO_TOOLS_SIGNATURE,
            "crypto-tools",
            "Post-quantum crypto command helpers",
            "crypto",
            vec![file(
                "/usr/bin/crypto-tools",
                "crypto-tools",
                true,
                "#!warsh\ncrypto\n",
            )],
            Vec::new(),
        ),
        signed_package(
            NETWORK_UTILS_SIGNATURE,
            "network-utils",
            "Network helper scripts",
            "binary",
            vec![
                file("/usr/bin/net-status", "net-status", true, "#!warsh\nnet status\n"),
                file(
                    "/usr/bin/net-dns",
                    "net-dns",
                    true,
                    "#!warsh\ndns warenterprise.com\n",
                ),
            ],
            Vec::new(),
        ),
        signed_package(
            SYSTEM_MONITOR_SIGNATURE,
            "system-monitor",
            "Top-style system dashboard launcher",
            "binary",
            vec![file(
                "/usr/bin/system-monitor",
                "system-monitor",
                true,
                "#!warsh\ntop\n",
            )],
            Vec::new(),
        ),
        signed_package(
            WAROS_DOCS_SIGNATURE,
            "waros-docs",
            "Offline WarOS documentation",
            "docs",
            vec![file(
                "/usr/share/doc/waros/README.txt",
                "README.txt",
                false,
                "WarOS package repository bootstrap.\nUse 'warpkg list', 'warpkg info <name>', and 'warpkg verify <name>'.\n",
            )],
            Vec::new(),
        ),
        signed_package(
            QUANTUM_BENCHMARKS_SIGNATURE,
            "quantum-benchmarks",
            "Quantum benchmark scripts",
            "quantum",
            vec![file(
                "/usr/share/quantum/benchmarks.txt",
                "benchmarks.txt",
                false,
                "bell: 2 qubits\nqft4: 4 qubits\nghz5: 5 qubits\n",
            )],
            vec![String::from("quantum-examples")],
        ),
        signed_package(
            HELLO_WORLD_SIGNATURE,
            "hello-world",
            "WarExec hello-world launcher",
            "binary",
            vec![file(
                "/usr/bin/hello",
                "hello",
                true,
                "#!warsh\necho Hello from WarOS package manager\n",
            )],
            Vec::new(),
        ),
        signed_package(
            WAR_SHELL_PLUGINS_SIGNATURE,
            "war-shell-plugins",
            "Shell helper launchers",
            "binary",
            vec![
                file("/usr/bin/waros-help", "waros-help", true, "#!warsh\nhelp\n"),
                file(
                    "/usr/bin/waros-version",
                    "waros-version",
                    true,
                    "#!warsh\nversion --all\n",
                ),
            ],
            Vec::new(),
        ),
    ]
}

fn signed_package(
    signature_hex: &str,
    name: &str,
    description: &str,
    category: &str,
    files: Vec<(ManifestFile, WarPackPayload)>,
    dependencies: Vec<String>,
) -> WarPackBundle {
    let (manifest_files, payloads): (Vec<_>, Vec<_>) = files.into_iter().unzip();
    let manifest = Manifest {
        name: name.into(),
        version: String::from("0.1.0"),
        description: description.into(),
        author: String::from("War Enterprise"),
        license: String::from("Apache-2.0"),
        files: manifest_files,
        dependencies,
        min_waros_version: crate::KERNEL_VERSION.into(),
        category: category.into(),
    };
    let digests = payload_digests(&payloads);
    WarPackBundle {
        signed_manifest: SignedManifest {
            manifest,
            payloads: digests,
            signature: SignatureEnvelope {
                scheme: String::from(WARPKG_SIGNATURE_SCHEME),
                key_id: String::from(WARPKG_BOOTSTRAP_KEY_ID),
                signature: String::from(signature_hex),
            },
        },
        payloads,
    }
}

fn file(path: &str, source: &str, executable: bool, contents: &str) -> (ManifestFile, WarPackPayload) {
    (
        ManifestFile {
            path: path.into(),
            source: source.into(),
            executable,
            size: contents.len() as u64,
        },
        WarPackPayload {
            source: source.into(),
            contents: contents.into(),
        },
    )
}
