Extra CA certificates for the image build and runtime, for networks that
intercept TLS. Drop `*.crt` / `*.pem` files here; they are gitignored. The
directory itself is tracked so the Dockerfile's `COPY .docker-certs/` has
something to copy on a plain checkout.
