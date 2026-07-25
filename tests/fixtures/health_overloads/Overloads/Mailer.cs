namespace Overloads;

// Overloads collapse into one symbol, so `Send` would otherwise be reported
// twice under the same name. The static and instance constructors share a name
// too, but neither is an overload of the other and neither should be suffixed.
public class Mailer
{
    static Mailer() { }

    public Mailer() { }

    public void Send(string message) { }

    public void Send(string message, int retries) { }

    public void Close() { }
}
