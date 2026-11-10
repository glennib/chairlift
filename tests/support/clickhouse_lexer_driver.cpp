// Prints ClickHouse's tokens for each input, using the standalone build of its
// lexer (src/Parsers/Lexer.cpp with LEXER_STANDALONE_BUILD).
//
// Reads stdin as inputs separated by NUL bytes. For each input it prints one
// line per token, "<TokenType value> <begin> <end>" with byte offsets into that
// input, followed by a line "-".

#include <Parsers/clickhouse_lexer.h>

#include <cstdio>
#include <cstdlib>
#include <vector>

int main()
{
    std::vector<char> input;
    char buffer[1 << 16];
    size_t n;
    while ((n = std::fread(buffer, 1, sizeof buffer, stdin)) > 0)
        input.insert(input.end(), buffer, buffer + n);

    void * lexer = std::malloc(clickhouse_lexer_size);
    const char * begin = input.data();
    const char * const end = input.data() + input.size();
    while (true)
    {
        const char * chunk_end = begin;
        while (chunk_end < end && *chunk_end != '\0')
            ++chunk_end;

        clickhouse_lexer_create(lexer, begin, chunk_end, 0);
        while (true)
        {
            const char * token_begin;
            const char * token_end;
            unsigned char type = clickhouse_lexer_next_token(lexer, &token_begin, &token_end);
            if (clickhouse_lexer_token_is_end(type))
                break;
            std::printf("%u %zu %zu\n", type, size_t(token_begin - begin), size_t(token_end - begin));
        }
        std::printf("-\n");

        if (chunk_end >= end)
            break;
        begin = chunk_end + 1;
    }
    std::free(lexer);
    return 0;
}
