# stm32-udp-client-rs
Running UDP client on STM32H7 using Embassy and Rust. 

## Run
To test the application, open two terminals: one for running the server and another for running this application.

```bash
$ nc -l 8000
```

```bash
cargo run 
```