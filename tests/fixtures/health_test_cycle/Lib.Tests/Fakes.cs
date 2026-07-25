namespace Lib.Tests;

// A cycle that lives entirely inside the test project, mirroring the one in
// Lib. `--exclude-tests` must drop this one and keep that one.
public class FakeAlpha
{
    public FakeBeta? Next;
}

public class FakeBeta
{
    public FakeAlpha? Next;
}
