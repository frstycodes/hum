package com.frstycodes.hum;

import java.io.Closeable;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.Base64;

import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;

/**
 * The smallest WebSocket client Hum needs: connect over TLS, send text and binary
 * frames. Hum's certificate is self-signed, so instead of a certificate authority the
 * client checks its SHA-256 fingerprint: the one the phone page handed over when
 * pairing, or (paired by hand) the one seen on the first connection.
 */
final class Ws implements Closeable {
    private final SSLSocket socket;
    private final OutputStream out;
    private final SecureRandom random = new SecureRandom();
    /** The fingerprint the PC presented (hex, lowercase). */
    final String fingerprint;

    private Ws(SSLSocket socket, String fingerprint) throws IOException {
        this.socket = socket;
        this.out = socket.getOutputStream();
        this.fingerprint = fingerprint;
    }

    static final class WrongPc extends IOException {
        WrongPc() {
            super("This isn't the Hum you paired with");
        }
    }

    static Ws connect(String host, int port, String pinned) throws IOException {
        final String[] seen = new String[1];
        TrustManager trust = new X509TrustManager() {
            @Override
            public void checkClientTrusted(X509Certificate[] chain, String auth) throws CertificateException {
                throw new CertificateException("no client certs");
            }

            @Override
            public void checkServerTrusted(X509Certificate[] chain, String auth) throws CertificateException {
                if (chain == null || chain.length == 0) throw new CertificateException("no certificate");
                seen[0] = sha256(chain[0].getEncoded());
                if (pinned != null && !pinned.isEmpty() && !pinned.equalsIgnoreCase(seen[0])) {
                    throw new CertificateException("fingerprint mismatch");
                }
            }

            @Override
            public X509Certificate[] getAcceptedIssuers() {
                return new X509Certificate[0];
            }
        };
        SSLSocket s;
        try {
            SSLContext ctx = SSLContext.getInstance("TLS");
            ctx.init(null, new TrustManager[] {trust}, null);
            Socket raw = new Socket();
            raw.connect(new InetSocketAddress(host, port), 5000);
            raw.setTcpNoDelay(true);
            raw.setSoTimeout(8000);
            s = (SSLSocket) ctx.getSocketFactory().createSocket(raw, host, port, true);
            s.startHandshake();
        } catch (javax.net.ssl.SSLHandshakeException e) {
            if (seen[0] != null) throw new WrongPc();
            throw e;
        } catch (java.security.GeneralSecurityException e) {
            throw new IOException(e);
        }

        byte[] key = new byte[16];
        new SecureRandom().nextBytes(key);
        String request = "GET /ws HTTP/1.1\r\n"
                + "Host: " + host + ":" + port + "\r\n"
                + "Upgrade: websocket\r\n"
                + "Connection: Upgrade\r\n"
                + "Sec-WebSocket-Key: " + Base64.getEncoder().encodeToString(key) + "\r\n"
                + "Sec-WebSocket-Version: 13\r\n\r\n";
        OutputStream o = s.getOutputStream();
        o.write(request.getBytes(StandardCharsets.US_ASCII));
        o.flush();

        // read the response head (up to the blank line) and check for 101
        InputStream in = s.getInputStream();
        StringBuilder head = new StringBuilder();
        int b;
        while ((b = in.read()) != -1) {
            head.append((char) b);
            int n = head.length();
            if (n >= 4 && head.charAt(n - 4) == '\r' && head.charAt(n - 3) == '\n' && head.charAt(n - 2) == '\r' && head.charAt(n - 1) == '\n') break;
            if (n > 8192) break;
        }
        if (!head.toString().startsWith("HTTP/1.1 101")) {
            s.close();
            throw new IOException("Hum didn't accept the connection");
        }
        s.setSoTimeout(0);
        return new Ws(s, seen[0]);
    }

    void sendText(String text) throws IOException {
        send(0x1, text.getBytes(StandardCharsets.UTF_8), -1);
    }

    void sendBinary(byte[] data, int length) throws IOException {
        send(0x2, data, length);
    }

    /** One masked frame (clients must mask), written in a single call. */
    private void send(int opcode, byte[] data, int length) throws IOException {
        int len = length < 0 ? data.length : length;
        int headLen = 2 + (len < 126 ? 0 : 2) + 4;
        byte[] frame = new byte[headLen + len];
        int i = 0;
        frame[i++] = (byte) (0x80 | opcode);
        if (len < 126) {
            frame[i++] = (byte) (0x80 | len);
        } else {
            frame[i++] = (byte) (0x80 | 126);
            frame[i++] = (byte) (len >> 8);
            frame[i++] = (byte) len;
        }
        byte[] mask = new byte[4];
        random.nextBytes(mask);
        System.arraycopy(mask, 0, frame, i, 4);
        i += 4;
        for (int j = 0; j < len; j++) frame[i + j] = (byte) (data[j] ^ mask[j & 3]);
        out.write(frame);
    }

    @Override
    public void close() {
        try {
            send(0x8, new byte[] {0x03, (byte) 0xE8}, -1); // 1000: normal close
        } catch (IOException ignored) {
        }
        try {
            socket.close();
        } catch (IOException ignored) {
        }
    }

    static String sha256(byte[] bytes) {
        try {
            byte[] d = MessageDigest.getInstance("SHA-256").digest(bytes);
            StringBuilder sb = new StringBuilder();
            for (byte x : d) sb.append(String.format("%02x", x));
            return sb.toString();
        } catch (Exception e) {
            return "";
        }
    }
}
