//! Test-only TLS termination for the counted Axum provider on an owned loopback port.
//! The model gateway and persisted routing catalogue use this same HTTPS origin.
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const PROXY: &str = r#"
import http.client,http.server,ssl,sys
from pathlib import Path
root=Path(sys.argv[1]); backend_port=int(sys.argv[2])
class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*args): pass
    def do_POST(self):
        body=self.rfile.read(int(self.headers.get('Content-Length','0')))
        headers={k:v for k,v in self.headers.items()
                 if k.lower() not in ('host','connection','content-length','transfer-encoding')}
        upstream=http.client.HTTPConnection('127.0.0.1',backend_port,timeout=5)
        try:
            upstream.request('POST',self.path,body=body,headers=headers)
            response=upstream.getresponse()
            payload=response.read()
            self.send_response(response.status)
            for key,value in response.getheaders():
                if key.lower() not in ('connection','content-length','transfer-encoding','server','date'):
                    self.send_header(key,value)
            self.send_header('Content-Length',str(len(payload)))
            self.send_header('Connection','close')
            self.end_headers()
            self.wfile.write(payload)
            self.close_connection=True
        finally:
            upstream.close()
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
server.daemon_threads=True
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root/'leaf.pem',root/'leaf.key')
server.socket=context.wrap_socket(server.socket,server_side=True)
print(server.server_port,flush=True)
server.serve_forever()
"#;

struct PrivateDirectory(PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) struct TlsProxy {
    pub(super) endpoint: String,
    pub(super) root: reqwest::Certificate,
    child: Child,
    _directory: PrivateDirectory,
}
impl Drop for TlsProxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn openssl(directory: &Path, args: &[&str]) {
    let status = Command::new("openssl")
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl is required by the real HTTPS confirmation fixture");
    assert!(
        status.success(),
        "isolated TLS certificate generation failed"
    );
}

impl TlsProxy {
    pub(super) fn start(backend: &str) -> Self {
        let backend = reqwest::Url::parse(backend).unwrap();
        assert_eq!(backend.scheme(), "http");
        assert_eq!(backend.host_str(), Some("127.0.0.1"));
        assert_eq!(backend.path(), "/v1");
        let directory = std::env::temp_dir().join(format!(
            "lmm-confirm-tls-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let owned_directory = PrivateDirectory(directory.clone());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        openssl(
            &directory,
            &[
                "req",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-keyout",
                "ca.key",
                "-out",
                "ca.pem",
                "-subj",
                "/CN=LMM owned confirmation test CA",
                "-days",
                "1",
                "-sha256",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
            ],
        );
        openssl(
            &directory,
            &[
                "req",
                "-new",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-keyout",
                "leaf.key",
                "-out",
                "leaf.csr",
                "-subj",
                "/CN=127.0.0.1",
            ],
        );
        fs::write(
            directory.join("extensions.txt"),
            "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n",
        )
        .unwrap();
        openssl(
            &directory,
            &[
                "x509",
                "-req",
                "-in",
                "leaf.csr",
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-out",
                "leaf.pem",
                "-days",
                "1",
                "-sha256",
                "-extfile",
                "extensions.txt",
            ],
        );
        let root =
            reqwest::Certificate::from_pem(&fs::read(directory.join("ca.pem")).unwrap()).unwrap();
        let child = Command::new("python3")
            .args(["-u", "-c", PROXY])
            .arg(&directory)
            .arg(backend.port().unwrap().to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 is required by the real HTTPS confirmation fixture");
        // Own the child before reading its startup reply so unwinding cleans up.
        let mut proxy = Self {
            endpoint: String::new(),
            root,
            child,
            _directory: owned_directory,
        };
        let mut port = String::new();
        BufReader::new(proxy.child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        let port: u16 = port.trim().parse().expect("owned TLS proxy must start");
        proxy.endpoint = format!("https://127.0.0.1:{port}/v1");
        proxy
    }
}
