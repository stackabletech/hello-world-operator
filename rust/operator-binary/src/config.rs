use indoc::{formatdoc, indoc};

pub fn generate_index_html(recipient: &str, color: &str) -> String {
    formatdoc! {"
    <!DOCTYPE html>
    <html>
    <head>
    <title>Hello {recipient}!</title>
    <style>
    html {{ color-scheme: light dark; color: {color}}}
    body {{ width: 35em; margin: 0 auto;
    font-family: Tahoma, Verdana, Arial, sans-serif; }}
    </style>
    </head>
    <body>
    <h1>Hello {recipient}!</h1>
    <p>If you see this page, your HelloCluster was deployed successfully.</p>
    </body>
    </html>
    "}
}

pub fn generate_nginx_conf() -> String {
    indoc! {"
    user  nginx;
    worker_processes  auto;

    error_log  /var/log/nginx/error.log notice;
    pid        /var/run/nginx.pid;
    daemon     off;


    events {
        worker_connections  1024;
    }


    http {
        include       /etc/nginx/mime.types;
        default_type  application/octet-stream;

        log_format  main  '$remote_addr - $remote_user [$time_local] \"$request\" '
                        '$status $body_bytes_sent \"$http_referer\" '
                        '\"$http_user_agent\" \"$http_x_forwarded_for\"';

        access_log  /var/log/nginx/access.log  main;

        sendfile        on;

        keepalive_timeout  65;

        server {
            listen       8080;
            listen  [::]:8080;
            server_name  localhost;

            location / {
                root   /stackable/mount/config;
                index  index.html;
            }
        }
    }
    "}
    .to_string()
}
