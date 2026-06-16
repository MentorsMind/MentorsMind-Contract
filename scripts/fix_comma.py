import glob

files = glob.glob("contracts/*/src/lib.rs") + ["escrow/src/lib.rs"]

for file in files:
    with open(file, "r") as f:
        content = f.read()
    
    if ",,\n    HealthDashboard" in content:
        content = content.replace(",,\n    HealthDashboard", ",\n    HealthDashboard")
        with open(file, "w") as f:
            f.write(content)
        print(f"Fixed {file}")
