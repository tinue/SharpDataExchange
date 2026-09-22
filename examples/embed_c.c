/* Minimal C embedder: read a .bas file, tokenize it, print the payload as hex.
 *
 *   cc examples/embed_c.c -Iinclude -Ltarget/release -lsharpdx -o /tmp/embed_c
 *   /tmp/embed_c tests/fixtures/depreciation.bas
 *
 * On macOS you may need: -Wl,-rpath,target/release (for the .dylib) or link the
 * static lib explicitly: target/release/libsharpdx.a plus -lSystem.
 */
#include <stdio.h>
#include <stdlib.h>
#include "sharpdx.h"

static unsigned char *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) { perror("open"); exit(1); }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc(n);
    if (fread(buf, 1, n, f) != (size_t)n) { perror("read"); exit(1); }
    fclose(f);
    *len = (size_t)n;
    return buf;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <file.bas>\n", argv[0]);
        return 2;
    }
    printf("sharpdx %s\n", sde_version());

    size_t in_len = 0;
    unsigned char *in = read_file(argv[1], &in_len);

    uint8_t *out = NULL;
    size_t out_len = 0;
    int32_t rc = sde_tokenize(SDE_DEVICE_PC1500, /*with_header=*/1, SDE_SEGMENT_MARKER_WIRE, "sample",
                              in, in_len, &out, &out_len);
    if (rc != SDE_OK) {
        fprintf(stderr, "sde_tokenize failed (%d): %s\n", rc, sde_last_error());
        return 1;
    }

    printf("%zu bytes:\n", out_len);
    for (size_t i = 0; i < out_len; i++) {
        printf("%02x%s", out[i], (i % 16 == 15) ? "\n" : " ");
    }
    if (out_len % 16) printf("\n");

    sde_buf_free(out, out_len);
    free(in);
    return 0;
}
