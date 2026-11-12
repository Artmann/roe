namespace App;

// A type and its own nested type refer to each other. The nested type is part
// of the outer type's implementation, so this is not a cycle worth reporting.
internal sealed class PhaseClock
{
    private readonly List<Measure> _measures = [];

    public Measure Start(string name)
    {
        var measure = new Measure(this, name);
        _measures.Add(measure);

        return measure;
    }

    public int Count => _measures.Count;

    public readonly struct Measure(PhaseClock clock, string name)
    {
        public string Name => name;

        public int Index => clock.Count;
    }
}

// A real cycle that runs through a nested type: Scheduler.Job uses Worker, and
// Worker uses Scheduler. It must still be found.
internal sealed class Scheduler
{
    public Job Next(Worker worker) => new(worker);

    public sealed class Job(Worker worker)
    {
        public Worker Worker => worker;
    }
}

internal sealed class Worker
{
    public Scheduler? Owner { get; set; }
}

internal static class Program
{
    private static void Main()
    {
        var clock = new PhaseClock();
        Console.WriteLine(clock.Start("cook").Index);
        Console.WriteLine(new Scheduler().Next(new Worker()).Worker.Owner);
    }
}
