#include <stdio.h>
#include <stdlib.h>

#define MAX_BUF 4096

typedef enum {
    OK = 0,
    ERR_PARSE,
    ERR_IO,
} status_t;

typedef struct {
    int id;
    char name[64];
    status_t status;
} record_t;

static int counter = 0;

int parse_record(const char *line, record_t *out) {
    if (line == NULL || out == NULL) return ERR_PARSE;
    return OK;
}

void print_record(const record_t *r) {
    printf("[%d] %s (status=%d)\n", r->id, r->name, r->status);
}

int main(void) {
    record_t r = { .id = 1, .name = "foo", .status = OK };
    print_record(&r);
    return 0;
}
