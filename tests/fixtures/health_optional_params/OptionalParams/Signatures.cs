namespace OptionalParams;

public class Signatures
{
    /// Sixteen parameters, two of which a caller has to supply. The whole
    /// call site is `Sum(1, 2)`.
    public int Sum(int required1, int required2, int p3 = 0, int p4 = 0,
                   int p5 = 0, int p6 = 0, int p7 = 0, int p8 = 0, int p9 = 0,
                   int p10 = 0, int p11 = 0, int p12 = 0, int p13 = 0,
                   int p14 = 0, int p15 = 0, int p16 = 0)
    {
        return required1 + required2;
    }

    /// Six parameters, two of which are returns wearing a parameter's
    /// clothes.
    public bool TryGet(int a, int b, int c, int d, out int first, out int second)
    {
        first = a + b;
        second = c + d;

        return true;
    }

    /// A `params` array is omittable, so it does not add to the call-site
    /// burden either.
    public void Log(string message, params object[] arguments)
    {
    }

    /// Six genuinely required parameters. This is what the check is for.
    public void Configure(string host, int port, string user, string password,
                          int timeout, bool secure)
    {
    }
}

public static class Extensions
{
    /// The receiver, a `ref`, and an `in` are all supplied by the caller, so
    /// all three count. Six required in total.
    public static void Blend(this Signatures target, ref int accumulator,
                             in int weight, string label, int rounds,
                             bool normalize)
    {
    }
}
