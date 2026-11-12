namespace App;

internal ref struct Cursor(int count)
{
    private int _index = -1;

    public readonly int Index => _index;

    public bool Advance() => ++_index < count;
}

// A pattern-based enumerable: GetEnumerator returns something with MoveNext
// and Current, with no IEnumerable in sight. `foreach` calls all three, and
// Dispose if it exists, without naming any of them.
internal readonly struct Rows(int count)
{
    public Enumerator GetEnumerator() => new(count);

    public ref struct Enumerator
    {
        private Cursor _cursor;

        internal Enumerator(int count)
        {
            _cursor = new Cursor(count);
        }

        public readonly int Current => _cursor.Index;

        public bool MoveNext() => _cursor.Advance();

        public void Dispose()
        {
        }
    }
}

// The async form: `await foreach` calls GetAsyncEnumerator, MoveNextAsync,
// Current and DisposeAsync.
internal sealed class Pages
{
    public PageEnumerator GetAsyncEnumerator(CancellationToken token = default) => new();

    public sealed class PageEnumerator
    {
        private int _page;

        public int Current => _page;

        public ValueTask<bool> MoveNextAsync() => new(++_page < 3);

        public ValueTask DisposeAsync() => ValueTask.CompletedTask;
    }
}

internal static class Program
{
    private static async Task Main()
    {
        foreach (int row in new Rows(3))
        {
            Console.WriteLine(row);
        }

        await foreach (int page in new Pages())
        {
            Console.WriteLine(page);
        }
    }
}
